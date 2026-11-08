//! 进程采集的自有受控夹具；只接受明确模式，不读取用户配置或系统剪贴板。
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn stdout(bytes: &[u8]) {
    let mut stream = std::io::stdout().lock();
    stream.write_all(bytes).unwrap();
    stream.flush().unwrap();
}

fn milliseconds(value: &str) -> Duration {
    let value: u64 = value.parse().unwrap();
    assert!(value <= 30_000);
    Duration::from_millis(value)
}

fn descendants(root: &str, token: &str) {
    let root = Path::new(root);
    assert!(root.is_absolute() && root.is_dir());
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("capture-case-"));
    assert!(token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    // 失败时也有明确自然退出期限；测试不会根据PID记录去kill其他进程。
    let mut child_command = Command::new(std::env::current_exe().unwrap());
    child_command
        .args(["sleep", "15000"])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child_command.creation_flags(0x0800_0000); // 自有后代也不创建控制台窗口。
    }
    let child = child_command.spawn().unwrap();
    let mut record = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root.join("owned-pids.json"))
        .unwrap();
    writeln!(
        record,
        "{{\"schemaVersion\":1,\"token\":\"{token}\",\"fixture\":{},\"descendant\":{},\"startedAtUnixMs\":{started}}}",
        std::process::id(),
        child.id()
    )
    .unwrap();
    record.flush().unwrap();
    record.sync_all().unwrap();
    let mut ready = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root.join("fixture-ready"))
        .unwrap();
    ready.write_all(token.as_bytes()).unwrap();
    ready.flush().unwrap();
    thread::sleep(Duration::from_secs(15));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("warmup") => {
            assert_eq!(args.len(), 2);
            stdout(b"omnitrigger-process-capture-fixture-v1");
        }
        Some("q5-utf8-trim") => {
            assert_eq!(args.len(), 2);
            stdout("\u{3000} \t中文😀 \r\n".as_bytes());
            let mut stream = std::io::stderr().lock();
            stream.write_all("合成诊断\n".as_bytes()).unwrap();
            stream.flush().unwrap();
        }
        Some("dual") => {
            assert_eq!(args.len(), 4);
            let out: usize = args[2].parse().unwrap();
            let err: usize = args[3].parse().unwrap();
            assert!(out <= 1024 * 1024 && err <= 1024 * 1024);
            let writer = thread::spawn(move || stdout(&vec![b'a'; out]));
            let mut stream = std::io::stderr().lock();
            stream.write_all(&vec![b'b'; err]).unwrap();
            stream.flush().unwrap();
            writer.join().unwrap();
        }
        Some("invalid-utf8") => {
            assert_eq!(args.len(), 2);
            stdout(&[0xff]);
        }
        Some("exit") => {
            assert_eq!(args.len(), 3);
            stdout(b"controlled output");
            std::process::exit(args[2].parse().unwrap());
        }
        Some("sleep") => {
            assert_eq!(args.len(), 3);
            thread::sleep(milliseconds(&args[2]));
        }
        Some("cwd") => {
            assert_eq!(args.len(), 2);
            stdout(
                std::env::current_dir()
                    .unwrap()
                    .to_string_lossy()
                    .as_bytes(),
            );
        }
        Some("descendant") => {
            assert_eq!(args.len(), 4);
            descendants(&args[2], &args[3]);
        }
        _ => panic!("只接受明确的受控模式"),
    }
}
