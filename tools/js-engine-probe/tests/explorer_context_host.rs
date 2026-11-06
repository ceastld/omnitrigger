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
            panic!("Explorer上下文宿主未在短时限内退出");
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
                    "宿主没有发出 explorer.context RPC"
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
    assert_eq!(frame["method"], "explorer.context");
    (stream, frame)
}

#[test]
fn actual_script_engine_sync_context_roundtrip_and_empty_selection() {
    let (listener, native) = bridge();
    let worker = thread::spawn(move || {
        for (index, value) in [
            json!({"currentDirectory":"D:\\fixture","selectedPaths":["D:\\fixture\\A 😀.txt"]}),
            json!({"currentDirectory":null,"selectedPaths":[]}),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut stream, frame) = receive(&listener);
            assert_eq!(frame["id"], index + 1);
            assert_eq!(frame["params"], json!({}));
            writeln!(
                stream,
                "{}",
                json!({"id":frame["id"],"ok":true,"value":value})
            )
            .unwrap();
        }
    });
    let(success,response)=invoke("const first=ctx.desktop.explorerContext();const second=ctx.desktop.explorerContext();return{first,second,sync:typeof first.then==='undefined',frozen:Object.isFrozen(first)&&Object.isFrozen(first.selectedPaths)};",Some(native));
    worker.join().unwrap();
    assert!(success, "{response}");
    assert_eq!(response["ok"], true);
    assert_eq!(response["result"]["sync"], true);
    assert_eq!(response["result"]["frozen"], true);
    assert_eq!(
        response["result"]["second"],
        json!({"currentDirectory":null,"selectedPaths":[]})
    );
}
#[test]
fn actual_script_engine_native_error_is_read_only_and_private_body_is_removed() {
    let (listener, native) = bridge();
    let worker = thread::spawn(move || {
        let (mut stream, frame) = receive(&listener);
        assert_eq!(frame["params"], json!({}));
        writeln!(stream,"{}",json!({"id":frame["id"],"ok":false,"error":{"code":"target_changed","message":"PRIVATE synthetic","effect_state":"unknown","method":"explorer.context"}})).unwrap();
    });
    let(success,response)=invoke("try{ctx.desktop.explorerContext();return{bad:true};}catch(e){return{code:e.code,effect:e.effectState,private:e.message.includes('PRIVATE')};}",Some(native));
    worker.join().unwrap();
    assert!(success, "{response}");
    assert_eq!(
        response["result"],
        json!({"code":"target_changed","effect":"not_started","private":false})
    );
}
