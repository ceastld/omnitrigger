//! 两引擎共用的真实轻量宿主测试；回环内存接收端不创建任何 UI 或读取个人配置。
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn invoke(script: &str, native: Option<Value>) -> (bool, Value) {
    let mut request = json!({"script":script,"input":{}});
    if let Some(native) = native {
        request["native_context"] = native;
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_omnitrigger-js-probe"));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("通知宿主未在 10 秒内退出");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !response.to_string().contains(TOKEN),
        "宿主回执不能泄露连接凭据"
    );
    (output.status.success(), response)
}

fn receive(listener: &TcpListener) -> (TcpStream, Value) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "真实宿主未发出通知 RPC");
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    };
    // Windows 的 accept 可能继承监听器非阻塞属性；后续明确使用有界阻塞读取。
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut byte = [0u8];
    while !bytes.ends_with(b"\n") {
        assert!(bytes.len() < 32 * 1024, "通知测试请求超出界限");
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    let frame: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(frame["token"].as_str() == Some(TOKEN));
    assert_eq!(frame["method"], "ui.notify");
    (stream, frame)
}

fn bridge() -> (TcpListener, Value) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let native = json!({"connection":{"port":listener.local_addr().unwrap().port(),"token":TOKEN}});
    (listener, native)
}

#[test]
fn notify_real_host_uses_whitelist_exact_parameters_and_synchronous_receipts() {
    let (listener, native) = bridge();
    let worker = thread::spawn(move || {
        let mut params = Vec::new();
        for expected_id in 1..=2 {
            let (mut stream, frame) = receive(&listener);
            assert_eq!(frame["id"], expected_id);
            params.push(frame["params"].clone());
            writeln!(
                stream,
                "{}",
                json!({"id":expected_id,"ok":true,"value":{"submitted":true}})
            )
            .unwrap();
        }
        params
    });
    let began = Instant::now();
    let (ok, response) = invoke(
        r#"
        const first=ctx.ui.notify({message:'  正文\r\n\t😀 <b>纯文本</b>  '});
        const second=ctx.ui.notify({message:'完成',title:'标题',level:'success',placement:'center',durationMs:30000});
        return {first,second,promise:typeof first.then==='function',hidden:ctx.__nativeContext===undefined};
    "#,
        Some(native),
    );
    let params = worker
        .join()
        .unwrap_or_else(|_| panic!("原生测试服务退出；宿主回执：{response}"));
    assert!(ok, "{response}");
    assert_eq!(
        response["result"],
        json!({"first":{"submitted":true},"second":{"submitted":true},"promise":false,"hidden":true})
    );
    assert_eq!(
        params,
        vec![
            json!({"message":"  正文\r\n\t😀 <b>纯文本</b>  ","title":"OmniTrigger","level":"info","placement":"topRight","durationMs":3000}),
            json!({"message":"完成","title":"标题","level":"success","placement":"center","durationMs":30000}),
        ]
    );
    assert!(
        began.elapsed() < Duration::from_secs(10),
        "不等待通知显示时长"
    );
}

#[test]
fn notify_real_host_validates_before_attempting_ui_and_preserves_unicode_boundaries() {
    let script = r#"
        const invalid=[null,[],{}, {message:''},{message:' \r\n\t '}, {message:'x'.repeat(1025)},
          {message:'😀'.repeat(513)}, {message:'\ud800'},{message:'\udfff'}, {message:'a',title:'\ud800'},
          {message:'a',title:'A\nB'},{message:'a',title:'A\u2028B'},{message:'a',title:'x'.repeat(129)},
          {message:'a',level:null},{message:'a',placement:'topLeft'},{message:'a',durationMs:'3000'},
          {message:'a',durationMs:999},{message:'a',durationMs:30001},{message:'a',extra:1},
          {message:'a',[Symbol('extra')]:1}];
        let rejected=0;
        for (const options of invalid) {
          try {ctx.ui.notify(options);}
          catch(e) {if(e.code==='invalid_input'&&e.method==='ui.notify'&&e.effectState==='not_started')rejected++;}
        }
        let boundary;
        try {ctx.ui.notify({message:'😀'.repeat(512),title:'😀'.repeat(64),durationMs:1000});}
        catch(e) {boundary={code:e.code,method:e.method,effect:e.effectState};}
        return {rejected,total:invalid.length,boundary};
    "#;
    let (ok, response) = invoke(script, None);
    assert!(ok, "{response}");
    assert_eq!(response["result"]["rejected"], response["result"]["total"]);
    assert_eq!(
        response["result"]["boundary"],
        json!({"code":"ui_unavailable","method":"ui.notify","effect":"not_started"})
    );
}

#[test]
fn notify_real_host_without_application_ui_reports_ui_unavailable() {
    for native in [
        None,
        Some(
            json!({"error":{"code":"desktop_unavailable","message":"没有应用 UI","effect_state":"not_started"}}),
        ),
    ] {
        let (ok, response) = invoke("return ctx.ui.notify({message:'通用通知'});", native);
        assert!(!ok);
        assert_eq!(response["error"]["code"], "ui_unavailable");
        assert_eq!(response["error"]["method"], "ui.notify");
        assert_eq!(response["error"]["effect_state"], "not_started");
    }
}

#[test]
fn notify_real_host_preserves_native_faults_without_retry() {
    for (code, effect) in [
        ("ui_busy", "not_started"),
        ("ui_unavailable", "not_started"),
        ("cancelled", "not_started"),
        ("timeout", "unknown"),
    ] {
        let (listener, native) = bridge();
        let worker = thread::spawn(move || {
            let (mut stream, frame) = receive(&listener);
            writeln!(stream,"{}",json!({"id":frame["id"],"ok":false,"error":{"code":code,"message":"未确认","effect_state":effect,"method":"ui.notify","required_permissions":["test.permission"]}})).unwrap();
            drop(stream);
            // 保留监听直至短窗口结束，检查错误后没有自动重试。
            thread::sleep(Duration::from_millis(50));
            assert!(matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
        });
        let (ok, response) = invoke("return ctx.ui.notify({message:'通用通知'});", Some(native));
        worker
            .join()
            .unwrap_or_else(|_| panic!("原生测试服务退出；宿主回执：{response}"));
        assert!(!ok);
        assert_eq!(response["error"]["code"], code);
        assert_eq!(response["error"]["effect_state"], effect);
        assert_eq!(response["error"]["method"], "ui.notify");
        assert_eq!(
            response["error"]["required_permissions"],
            json!(["test.permission"])
        );
    }
}

#[test]
fn notify_native_rpc_has_a_short_transport_deadline() {
    let (listener, native) = bridge();
    let worker = thread::spawn(move || {
        let (mut stream, _) = receive(&listener);
        let mut byte = [0u8];
        // 不发送创建回执；真实客户端须在普通 RPC 时限退出，而不是等待选择窗口的 66 秒。
        assert_eq!(stream.read(&mut byte).unwrap(), 0);
        assert!(matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
    });
    let (ok, response) = invoke(
        "return ctx.ui.notify({message:'通用通知',durationMs:30000});",
        Some(native),
    );
    worker
        .join()
        .unwrap_or_else(|_| panic!("原生测试服务退出；宿主回执：{response}"));
    assert!(!ok);
    assert_eq!(response["error"]["code"], "native_bridge_failed");
    assert_eq!(response["error"]["method"], "ui.notify");
    assert_eq!(response["error"]["effect_state"], "unknown");
}
