//! 真实两引擎进程与内存 TCP 接收端；不查询测试路径、不打开内容或读取个人配置。
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const TOKEN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const PATH: &str = "X:\\test-only-path-marker\\directory";

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
    let deadline = Instant::now() + Duration::from_secs(12);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("目录枚举宿主未在短时限内退出");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        !response.to_string().contains(TOKEN),
        "宿主回执不能包含测试凭据"
    );
    (output.status.success(), response)
}

fn bridge() -> (TcpListener, Value) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let native = json!({"connection":{"port":listener.local_addr().unwrap().port(),"token":TOKEN}});
    (listener, native)
}

fn receive(listener: &TcpListener) -> (TcpStream, Value) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "宿主没有发出 fs.read_directory RPC"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    };
    // Windows accept 可能继承监听器的非阻塞属性。
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut byte = [0u8];
    while !bytes.ends_with(b"\n") {
        assert!(bytes.len() < 256 * 1024);
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    let frame: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(frame["token"].as_str() == Some(TOKEN));
    assert_eq!(frame["method"], "fs.read_directory");
    (stream, frame)
}

fn assert_fault(response: &Value, code: &str) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], code);
    assert_eq!(response["error"]["method"], "fs.read_directory");
    assert_eq!(response["error"]["effect_state"], "not_started");
    assert!(!response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("test-only-path-marker"));
}

#[test]
fn directory_real_sync_host_whitelist_and_exact_wire_round_trip() {
    let (listener, native) = bridge();
    let value = json!({"entries":[{"name":"file.txt","path":format!("{PATH}\\file.txt"),"kind":"file","reason":null}],"complete":true});
    let expected = value.clone();
    let worker = thread::spawn(move || {
        let (mut stream, frame) = receive(&listener);
        assert_eq!(frame["params"], json!({"path":PATH,"max_entries":2}));
        writeln!(
            stream,
            "{}",
            json!({"id":frame["id"],"ok":true,"value":value})
        )
        .unwrap();
    });
    let script = format!(
        "const value=ctx.fs.readDirectory({},{{maxEntries:2}});return {{value,promise:typeof value.then==='function'}};",
        serde_json::to_string(PATH).unwrap()
    );
    let (ok, response) = invoke(&script, Some(native));
    worker.join().unwrap();
    assert!(ok, "{response}");
    assert_eq!(
        response["result"],
        json!({"value":expected,"promise":false})
    );
}
#[test]
fn directory_real_sync_host_missing_bridge_and_invalid_input_remain_structured() {
    let (_, response) = invoke(
        &format!(
            "return ctx.fs.readDirectory({});",
            serde_json::to_string(PATH).unwrap()
        ),
        None,
    );
    assert_fault(&response, "fs_unavailable");
    let (ok, response) = invoke(
        r#"const errors=[];for(const options of [{maxEntries:0},{maxEntries:1025},{recursive:true},null]){try{ctx.fs.readDirectory('X:/synthetic/',options);}catch(e){errors.push([e.code,e.method,e.effectState]);}}return errors;"#,
        None,
    );
    assert!(ok, "{response}");
    assert_eq!(
        response["result"],
        json!(vec![
            json!([
                "invalid_input",
                "fs.read_directory",
                "not_started"
            ]);
            4
        ])
    );
    let (listener, native) = bridge();
    let worker = thread::spawn(move || {
        let (mut stream, frame) = receive(&listener);
        writeln!(stream,"{}",json!({"id":frame["id"],"ok":false,"error":{"code":"directory_too_large","message":"test-only-path-marker","effect_state":"unknown"}})).unwrap();
    });
    let (_, response) = invoke(
        &format!(
            "return ctx.fs.readDirectory({});",
            serde_json::to_string(PATH).unwrap()
        ),
        Some(native),
    );
    worker.join().unwrap();
    assert_fault(&response, "directory_too_large");
}
