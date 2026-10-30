//! 亮度 HUD 的 CPU 离屏绘制；不创建窗口、读取显示器或查询亮度。
use std::{ffi::c_void, mem::size_of, ptr};
use windows::{
    core::{w, PCWSTR},
    Win32::Graphics::{
        Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
        },
        GdiPlus as gp,
    },
};

const LOGICAL_WIDTH: f32 = 228.;
const LOGICAL_HEIGHT: f32 = 76.;
// GDI+ 的 PixelFormat32bppPARGB：32 位、GDI、alpha 和预乘 alpha。
const PIXEL_FORMAT_PARGB: i32 =
    (11 | (32 << 8) | gp::PixelFormatGDI | gp::PixelFormatAlpha | gp::PixelFormatPAlpha) as i32;

fn check(status: gp::Status, operation: &str) -> Result<(), String> {
    if status.0 == 0 {
        Ok(())
    } else {
        Err(format!("HUD {operation}失败：GDI+ 状态 {}。", status.0))
    }
}

/// DIB 中始终保存适合 UpdateLayeredWindow 的 BGRA 预乘像素。
/// 位图、DC 和 GDI+ 对象只由创建它们的 HUD 线程使用。
pub(super) struct HudBitmap {
    token: usize,
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    width: i32,
    height: i32,
    image: *mut gp::GpBitmap,
    graphics: *mut gp::GpGraphics,
    family: *mut gp::GpFontFamily,
    font: *mut gp::GpFont,
    font_scale: f32,
    format: *mut gp::GpStringFormat,
    base: Vec<u32>,
    base_scale: f32,
}

impl HudBitmap {
    pub(super) fn new(width: i32, height: i32) -> Result<Self, String> {
        if width <= 0 || height <= 0 || width > 8192 || height > 8192 {
            return Err("HUD 位图尺寸无效。".into());
        }
        let mut token = 0;
        let input = gp::GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        check(
            unsafe { gp::GdiplusStartup(&mut token, &input, ptr::null_mut()) },
            "初始化绘图",
        )?;
        let mut result = Self {
            token,
            dc: HDC::default(),
            bitmap: HBITMAP::default(),
            previous: HGDIOBJ::default(),
            bits: ptr::null_mut(),
            width,
            height,
            image: ptr::null_mut(),
            graphics: ptr::null_mut(),
            family: ptr::null_mut(),
            font: ptr::null_mut(),
            font_scale: 0.,
            format: ptr::null_mut(),
            base: Vec::new(),
            base_scale: 0.,
        };
        result.dc = unsafe { CreateCompatibleDC(None) };
        if result.dc.is_invalid() {
            return Err(format!(
                "无法创建 HUD 离屏 DC：{}",
                std::io::Error::last_os_error()
            ));
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = ptr::null_mut();
        result.bitmap =
            unsafe { CreateDIBSection(Some(result.dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) }
                .map_err(|error| format!("无法创建 HUD 透明位图：{error}"))?;
        if bits.is_null() {
            return Err("HUD 位图没有可写像素。".into());
        }
        result.bits = bits.cast();
        result.previous = unsafe { SelectObject(result.dc, HGDIOBJ(result.bitmap.0)) };
        if result.previous.is_invalid() || result.previous.0 as isize == -1 {
            result.previous = HGDIOBJ::default();
            return Err("无法选择 HUD 离屏位图。".into());
        }
        unsafe { ptr::write_bytes(result.bits, 0, width as usize * height as usize) };
        check(
            unsafe {
                gp::GdipCreateBitmapFromScan0(
                    width,
                    height,
                    width * 4,
                    PIXEL_FORMAT_PARGB,
                    Some(result.bits.cast()),
                    &mut result.image,
                )
            },
            "绑定透明位图",
        )?;
        check(
            unsafe { gp::GdipGetImageGraphicsContext(result.image.cast(), &mut result.graphics) },
            "创建绘图上下文",
        )?;
        check(
            unsafe { gp::GdipSetSmoothingMode(result.graphics, gp::SmoothingModeAntiAlias8x8) },
            "设置形状抗锯齿",
        )?;
        check(
            unsafe { gp::GdipSetPixelOffsetMode(result.graphics, gp::PixelOffsetModeHalf) },
            "设置像素对齐",
        )?;
        check(
            unsafe {
                gp::GdipSetTextRenderingHint(result.graphics, gp::TextRenderingHintAntiAliasGridFit)
            },
            "设置文字抗锯齿",
        )?;
        check(
            unsafe { gp::GdipSetCompositingMode(result.graphics, gp::CompositingModeSourceOver) },
            "设置透明合成",
        )?;
        check(
            unsafe {
                gp::GdipCreateFontFamilyFromName(
                    w!("Segoe UI"),
                    ptr::null_mut(),
                    &mut result.family,
                )
            },
            "加载字体",
        )?;
        check(
            unsafe {
                gp::GdipCreateStringFormat(
                    gp::StringFormatFlagsNoWrap.0 | gp::StringFormatFlagsNoClip.0,
                    0,
                    &mut result.format,
                )
            },
            "创建文字布局",
        )?;
        check(
            unsafe { gp::GdipSetStringFormatAlign(result.format, gp::StringAlignmentFar) },
            "设置数字对齐",
        )?;
        check(
            unsafe { gp::GdipSetStringFormatLineAlign(result.format, gp::StringAlignmentCenter) },
            "设置文字居中",
        )?;
        Ok(result)
    }

    pub(super) fn hdc(&self) -> HDC {
        self.dc
    }

    pub(super) fn render(&mut self, percent: Option<u8>, scale: f32) -> Result<(), String> {
        if !scale.is_finite() || scale <= 0. || scale > 32. {
            return Err("HUD 绘制缩放无效。".into());
        }
        if self.font.is_null() || self.font_scale != scale {
            let mut font = ptr::null_mut();
            check(
                unsafe {
                    gp::GdipCreateFont(
                        self.family,
                        13. * scale,
                        gp::FontStyleRegular.0,
                        gp::UnitPixel,
                        &mut font,
                    )
                },
                "创建数字字体",
            )?;
            if !self.font.is_null() {
                let _ = unsafe { gp::GdipDeleteFont(self.font) };
            }
            self.font = font;
            self.font_scale = scale;
        }
        // 写 DIB 之前完成 GDI+ 命令。同一尺寸和 DPI 的静态外观只绘制一次。
        check(
            unsafe { gp::GdipFlush(self.graphics, gp::FlushIntentionSync) },
            "完成上一帧",
        )?;
        if self.base_scale != scale {
            self.cache_base(scale)?;
        }
        // 恢复不含数字与进度的底图，清除上一帧所有动态像素。
        unsafe { ptr::copy_nonoverlapping(self.base.as_ptr(), self.bits, self.base.len()) };
        if let Some(percent) = percent.filter(|percent| *percent > 0) {
            let width = 108. * f32::from(percent.min(100)) / 100.;
            let fill = Path::rounded(
                54. * scale,
                36. * scale,
                width * scale,
                4. * scale,
                (width * 0.5).min(2.) * scale,
            )?;
            self.fill(&fill, 0xffdb9ee5)?;
        }
        let text = percent.map_or_else(|| "—".into(), |percent| percent.min(100).to_string());
        let text: Vec<u16> = text.encode_utf16().collect();
        let layout = gp::RectF {
            X: 174. * scale,
            Y: 14. * scale,
            Width: 26. * scale,
            Height: 48. * scale,
        };
        let foreground = Brush::new(0xfff5f5f5)?;
        check(
            unsafe {
                gp::GdipDrawString(
                    self.graphics,
                    PCWSTR(text.as_ptr()),
                    text.len() as i32,
                    self.font,
                    &layout,
                    self.format,
                    foreground.0.cast(),
                )
            },
            "绘制数字",
        )?;
        check(
            unsafe { gp::GdipFlush(self.graphics, gp::FlushIntentionSync) },
            "完成透明帧",
        )
    }

    fn fill(&self, path: &Path, color: u32) -> Result<(), String> {
        let brush = Brush::new(color)?;
        check(
            unsafe { gp::GdipFillPath(self.graphics, brush.0.cast(), path.0) },
            "填充形状",
        )
    }

    fn cache_base(&mut self, scale: f32) -> Result<(), String> {
        self.paint_shadow(scale);
        let card = Path::rounded(
            14. * scale,
            14. * scale,
            200. * scale,
            48. * scale,
            8. * scale,
        )?;
        self.fill(&card, 0xff2c2c2c)?;
        let border = Path::rounded(
            14.5 * scale,
            14.5 * scale,
            199. * scale,
            47. * scale,
            7.5 * scale,
        )?;
        let border_pen = Pen::new(0xff242424, scale)?;
        check(
            unsafe { gp::GdipDrawPath(self.graphics, border_pen.0, border.0) },
            "绘制卡片边线",
        )?;
        self.paint_sun(scale)?;
        let track = Path::rounded(
            54. * scale,
            36. * scale,
            108. * scale,
            4. * scale,
            2. * scale,
        )?;
        self.fill(&track, 0xff9f9f9f)?;
        check(
            unsafe { gp::GdipFlush(self.graphics, gp::FlushIntentionSync) },
            "缓存静态外观",
        )?;
        let pixels = unsafe {
            std::slice::from_raw_parts(self.bits, self.width as usize * self.height as usize)
        };
        self.base.clear();
        self.base.extend_from_slice(pixels);
        self.base_scale = scale;
        Ok(())
    }

    fn paint_sun(&self, scale: f32) -> Result<(), String> {
        let pen = Pen::new(0xfff5f5f5, 1.2 * scale)?;
        let _ = unsafe {
            gp::GdipSetPenLineCap197819(pen.0, gp::LineCapRound, gp::LineCapRound, gp::DashCapRound)
        };
        let (cx, cy) = (35. * scale, 38. * scale);
        check(
            unsafe {
                gp::GdipDrawEllipse(
                    self.graphics,
                    pen.0,
                    cx - 3. * scale,
                    cy - 3. * scale,
                    6. * scale,
                    6. * scale,
                )
            },
            "绘制亮度图标",
        )?;
        for ray in 0..8 {
            let angle = ray as f32 * std::f32::consts::FRAC_PI_4;
            let (sin, cos) = angle.sin_cos();
            check(
                unsafe {
                    gp::GdipDrawLine(
                        self.graphics,
                        pen.0,
                        cx + cos * 5. * scale,
                        cy + sin * 5. * scale,
                        cx + cos * 7. * scale,
                        cy + sin * 7. * scale,
                    )
                },
                "绘制亮度光线",
            )?;
        }
        Ok(())
    }

    fn paint_shadow(&mut self, scale: f32) {
        let pixels = unsafe {
            std::slice::from_raw_parts_mut(self.bits, self.width as usize * self.height as usize)
        };
        // 3px 的极轻高斯外影。黑色的预乘 RGB 为零，仅写 alpha；14px 留白足以完全衰减。
        for y in 0..self.height {
            let py = (y as f32 + 0.5) / scale;
            for x in 0..self.width {
                let px = (x as f32 + 0.5) / scale;
                let qx = (px - LOGICAL_WIDTH * 0.5).abs() - (100. - 8.);
                let qy = (py - (LOGICAL_HEIGHT * 0.5 + 1.)).abs() - (24. - 8.);
                let distance = qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - 8.;
                let alpha = if distance <= 0. {
                    26
                } else if distance < 10. {
                    (26. * (-0.5 * (distance / 3.).powi(2)).exp()).round() as u32
                } else {
                    0
                };
                pixels[(y * self.width + x) as usize] = alpha << 24;
            }
        }
    }
}

impl Drop for HudBitmap {
    fn drop(&mut self) {
        unsafe {
            if !self.font.is_null() {
                let _ = gp::GdipDeleteFont(self.font);
            }
            if !self.family.is_null() {
                let _ = gp::GdipDeleteFontFamily(self.family);
            }
            if !self.format.is_null() {
                let _ = gp::GdipDeleteStringFormat(self.format);
            }
            if !self.graphics.is_null() {
                let _ = gp::GdipDeleteGraphics(self.graphics);
            }
            if !self.image.is_null() {
                let _ = gp::GdipDisposeImage(self.image.cast());
            }
            if !self.previous.is_invalid() {
                let _ = SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
            if !self.dc.is_invalid() {
                let _ = DeleteDC(self.dc);
            }
            gp::GdiplusShutdown(self.token);
        }
    }
}

struct Path(*mut gp::GpPath);
impl Path {
    fn rounded(x: f32, y: f32, width: f32, height: f32, radius: f32) -> Result<Self, String> {
        let mut raw = ptr::null_mut();
        check(
            unsafe { gp::GdipCreatePath(gp::FillModeAlternate, &mut raw) },
            "创建圆角路径",
        )?;
        let path = Self(raw);
        let diameter = radius * 2.;
        for (x, y, start) in [
            (x, y, 180.),
            (x + width - diameter, y, 270.),
            (x + width - diameter, y + height - diameter, 0.),
            (x, y + height - diameter, 90.),
        ] {
            check(
                unsafe { gp::GdipAddPathArc(path.0, x, y, diameter, diameter, start, 90.) },
                "构造圆角",
            )?;
        }
        check(unsafe { gp::GdipClosePathFigure(path.0) }, "闭合圆角路径")?;
        Ok(path)
    }
}
impl Drop for Path {
    fn drop(&mut self) {
        let _ = unsafe { gp::GdipDeletePath(self.0) };
    }
}

struct Brush(*mut gp::GpSolidFill);
impl Brush {
    fn new(color: u32) -> Result<Self, String> {
        let mut raw = ptr::null_mut();
        check(
            unsafe { gp::GdipCreateSolidFill(color, &mut raw) },
            "创建画刷",
        )?;
        Ok(Self(raw))
    }
}
impl Drop for Brush {
    fn drop(&mut self) {
        let _ = unsafe { gp::GdipDeleteBrush(self.0.cast()) };
    }
}

struct Pen(*mut gp::GpPen);
impl Pen {
    fn new(color: u32, width: f32) -> Result<Self, String> {
        let mut raw = ptr::null_mut();
        check(
            unsafe { gp::GdipCreatePen1(color, width, gp::UnitPixel, &mut raw) },
            "创建画笔",
        )?;
        Ok(Self(raw))
    }
}
impl Drop for Pen {
    fn drop(&mut self) {
        let _ = unsafe { gp::GdipDeletePen(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(bitmap: &HudBitmap, x: i32, y: i32) -> u32 {
        unsafe { *bitmap.bits.add((y * bitmap.width + x) as usize) }
    }

    fn snapshot(bitmap: &HudBitmap) -> Vec<u32> {
        unsafe {
            std::slice::from_raw_parts(bitmap.bits, bitmap.width as usize * bitmap.height as usize)
                .to_vec()
        }
    }

    #[test]
    fn rendered_frames_have_transparent_margins_and_premultiplied_antialiasing() {
        for scale in [1., 1.25, 2.] {
            let mut bitmap = HudBitmap::new((228. * scale) as i32, (76. * scale) as i32).unwrap();
            for percent in [None, Some(0), Some(50), Some(100), Some(255)] {
                bitmap.render(percent, scale).unwrap();
                assert_eq!(pixel(&bitmap, 0, 0), 0);
                assert_eq!(pixel(&bitmap, bitmap.width - 1, bitmap.height - 1), 0);
                assert_eq!(
                    pixel(&bitmap, (20. * scale) as i32, (38. * scale) as i32),
                    0xff2c2c2c
                );
                let pixels = unsafe {
                    std::slice::from_raw_parts(
                        bitmap.bits,
                        bitmap.width as usize * bitmap.height as usize,
                    )
                };
                assert!(pixels.iter().any(|pixel| (1..255).contains(&(pixel >> 24))));
                for &pixel in pixels {
                    let alpha = pixel >> 24;
                    assert!((pixel & 255) <= alpha);
                    assert!(((pixel >> 8) & 255) <= alpha);
                    assert!(((pixel >> 16) & 255) <= alpha);
                }
            }
        }
    }

    #[test]
    fn confirmed_percentage_changes_the_track_without_retaining_the_previous_frame() {
        let mut bitmap = HudBitmap::new(228, 76).unwrap();
        bitmap.render(None, 1.).unwrap();
        assert_eq!(pixel(&bitmap, 90, 38), 0xff9f9f9f);
        let unknown = snapshot(&bitmap);
        bitmap.render(Some(0), 1.).unwrap();
        let zero = snapshot(&bitmap);
        bitmap.render(Some(50), 1.).unwrap();
        assert_eq!(pixel(&bitmap, 90, 38), 0xffdb9ee5);
        assert_eq!(pixel(&bitmap, 140, 38), 0xff9f9f9f);
        bitmap.render(Some(100), 1.).unwrap();
        assert_eq!(pixel(&bitmap, 140, 38), 0xffdb9ee5);
        bitmap.render(None, 1.).unwrap();
        assert_eq!(snapshot(&bitmap), unknown);
        bitmap.render(Some(0), 1.).unwrap();
        assert_eq!(snapshot(&bitmap), zero);
        assert!(bitmap.render(None, f32::NAN).is_err());
        assert!(bitmap.render(None, 0.).is_err());
        assert!(HudBitmap::new(0, 76).is_err());
        assert!(HudBitmap::new(228, -1).is_err());
    }

    #[test]
    fn changing_scale_rebuilds_the_static_frame_and_reuses_it_without_ghosts() {
        let mut fresh = HudBitmap::new(285, 95).unwrap();
        fresh.render(None, 1.25).unwrap();
        let expected = snapshot(&fresh);
        let mut changed = HudBitmap::new(285, 95).unwrap();
        changed.render(Some(100), 1.).unwrap();
        changed.render(None, 1.25).unwrap();
        assert_eq!(snapshot(&changed), expected);
        changed.render(Some(100), 1.25).unwrap();
        changed.render(None, 1.25).unwrap();
        assert_eq!(snapshot(&changed), expected);
    }

    #[test]
    #[ignore = "手动测量离屏热路径；不创建窗口或操作硬件"]
    fn warm_render_timing() {
        use std::time::Instant;
        for scale in [1., 1.25] {
            let mut bitmap = HudBitmap::new((228. * scale) as i32, (76. * scale) as i32).unwrap();
            for percent in 0..16 {
                bitmap.render(Some(percent), scale).unwrap();
            }
            let mut durations = Vec::with_capacity(200);
            for frame in 0..200 {
                let percent = (frame % 7 != 0).then_some((frame % 101) as u8);
                let started = Instant::now();
                bitmap.render(percent, scale).unwrap();
                durations.push(started.elapsed());
            }
            durations.sort_unstable();
            eprintln!(
                "HUD scale={scale}: 离屏热绘制 p50={:.3}ms p95={:.3}ms",
                durations[100].as_secs_f64() * 1000.,
                durations[190].as_secs_f64() * 1000.
            );
        }
    }
}
