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
const PATH: &str = "X:\\test-only-path-marker\\file.txt";

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
            panic!("文件状态宿主未在短时限内退出");
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
                assert!(Instant::now() < deadline, "宿主没有发出 fs.stat RPC");
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
    assert_eq!(frame["method"], "fs.stat");
    (stream, frame)
}

fn assert_fault(response: &Value, code: &str) {
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"]["code"], code);
    assert_eq!(response["error"]["method"], "fs.stat");
    assert_eq!(response["error"]["effect_state"], "not_started");
    assert!(!response["error"]["message"]
        .as_str()
        .unwrap()
        .contains("test-only-path-marker"));
}

#[test]
fn stat_real_host_whitelist_exact_path_and_all_four_kinds_round_trip() {
    let (listener, native) = bridge();
    let worker = thread::spawn(move || {
        let mut paths = Vec::new();
        for (index, kind) in ["file", "directory", "missing", "other"]
            .into_iter()
            .enumerate()
        {
            let (mut stream, frame) = receive(&listener);
            assert_eq!(frame["id"], index + 1);
            paths.push(frame["params"]["path"].as_str().unwrap().to_owned());
            writeln!(
                stream,
                "{}",
                json!({"id":frame["id"],"ok":true,"value":{"kind":kind}})
            )
            .unwrap();
        }
        paths
    });
    let script = format!(
        r#"
        const paths=[{},'C:/','z:/folder/', 'C:\\中文😀'];
        const values=paths.map(path=>ctx.fs.stat(path));
        return {{kinds:values.map(value=>value.kind),promise:typeof values[0].then==='function',hidden:ctx.__nativeContext===undefined,read:typeof ctx.fs.readText,write:typeof ctx.fs.writeText}};
    "#,
        serde_json::to_string(PATH).unwrap()
    );
    let (ok, response) = invoke(&script, Some(native));
    let paths = worker.join().unwrap();
    assert!(ok, "{response}");
    assert_eq!(
        response["result"],
        json!({"kinds":["file","directory","missing","other"],"promise":false,"hidden":true,"read":"function","write":"function"})
    );
    assert_eq!(paths, [PATH, "C:/", "z:/folder/", "C:\\中文😀"]);
}

#[test]
fn stat_real_host_checks_lexical_boundaries_before_requiring_a_native_host() {
    let script = r#"
        const invalid=[null,[],{},true,1,'','C:','C:relative','relative','/root','\\root',
          '\\\\server\\share','\\\\?\\C:\\x','\\\\.\\C:\\x','1:\\x',
          'C:\\a\\\\b','C:/a//b','C:\\.','C:\\..','C:\\a\\..\\b','C:\\a.','C:\\a ',
          'C:\\a:stream','C:\\a*','C:\\a?','C:\\a<','C:\\a>','C:\\a"','C:\\a|',
          'C:\\NUL.txt','C:\\COM9.txt','C:\\LPT1.x','C:\\CON .txt',
          'C:\\COM¹','C:\\COM².txt','C:\\COM³ .txt','C:\\LPT¹','C:\\LPT².txt','C:\\LPT³ .txt',
          'C:\\\ud800','C:\\\udc00','C:\\a\n','C:\\a\u007fb',
          'C:\\'+'a'.repeat(32765),'C:\\'+'😀'.repeat(16383)];
        let rejected=0;
        for(const path of invalid){try{ctx.fs.stat(path);}catch(e){if(e.code==='invalid_input'&&e.method==='fs.stat'&&e.effectState==='not_started')rejected++;}}
        const codes=[];
        for(const path of ['C:\\OneDrive-work\\x','C:/safe/DROPBOX/x','C:\\iCloudDocs\\x']){
          try{ctx.fs.stat(path);}catch(e){codes.push(e.code);}
        }
        const boundaries=[];
        for(const path of ['C:\\','z:/','C:\\a\\','C:\\'+'a'.repeat(32764),'C:\\'+'😀'.repeat(16382)]){
          try{ctx.fs.stat(path);}catch(e){boundaries.push(e.code);}
        }
        return {rejected,total:invalid.length,codes,boundaries};
    "#;
    let (ok, response) = invoke(script, None);
    assert!(ok, "{response}");
    assert_eq!(response["result"]["rejected"], response["result"]["total"]);
    assert_eq!(
        response["result"]["codes"],
        json!(vec!["path_unavailable"; 3])
    );
    assert_eq!(
        response["result"]["boundaries"],
        json!(vec!["fs_unavailable"; 5])
    );
}

#[test]
fn stat_real_host_no_native_host_is_fs_unavailable_without_echoing_cause() {
    for native in [
        None,
        Some(
            json!({"error":{"code":"desktop_unavailable","message":format!("{PATH} {TOKEN}"),"effect_state":"unknown"}}),
        ),
    ] {
        let (_, response) = invoke(
            &format!(
                "return ctx.fs.stat({});",
                serde_json::to_string(PATH).unwrap()
            ),
            native,
        );
        assert_fault(&response, "fs_unavailable");
    }
    let (listener, native) = bridge();
    drop(listener);
    let (_, response) = invoke(
        &format!(
            "return ctx.fs.stat({});",
            serde_json::to_string(PATH).unwrap()
        ),
        Some(native),
    );
    assert_fault(&response, "fs_unavailable");
}

#[test]
fn stat_real_host_preserves_read_only_errors_and_never_retries() {
    for code in [
        "invalid_input",
        "path_not_local",
        "path_unavailable",
        "access_denied",
        "fs_error",
        "fs_unavailable",
        "cancelled",
        "timeout",
    ] {
        let (listener, native) = bridge();
        let worker = thread::spawn(move || {
            let (mut stream, frame) = receive(&listener);
            writeln!(stream,"{}",json!({"id":frame["id"],"ok":false,"error":{"code":code,"message":format!("{PATH} {TOKEN}"),"effect_state":"unknown","method":"fs.stat"}})).unwrap();
            drop(stream);
            thread::sleep(Duration::from_millis(50));
            assert!(matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
        });
        let (_, response) = invoke(
            &format!(
                "return ctx.fs.stat({});",
                serde_json::to_string(PATH).unwrap()
            ),
            Some(native),
        );
        worker.join().unwrap();
        assert_fault(&response, code);
    }
}

#[test]
fn stat_real_host_malformed_result_or_response_is_not_missing() {
    for value in [
        json!({}),
        json!({"kind":"unknown"}),
        json!({"kind":"file","extra":true}),
        Value::Null,
    ] {
        let (listener, native) = bridge();
        let worker = thread::spawn(move || {
            let (mut stream, frame) = receive(&listener);
            writeln!(
                stream,
                "{}",
                json!({"id":frame["id"],"ok":true,"value":value})
            )
            .unwrap();
        });
        let (_, response) = invoke(
            &format!(
                "return ctx.fs.stat({});",
                serde_json::to_string(PATH).unwrap()
            ),
            Some(native),
        );
        worker.join().unwrap();
        assert_fault(&response, "fs_error");
    }
    let (listener, native) = bridge();
    let worker = thread::spawn(move || {
        let (mut stream, _) = receive(&listener);
        writeln!(
            stream,
            "{}",
            json!({"id":999,"ok":true,"value":{"kind":"missing"}})
        )
        .unwrap();
    });
    let (_, response) = invoke(
        &format!(
            "return ctx.fs.stat({});",
            serde_json::to_string(PATH).unwrap()
        ),
        Some(native),
    );
    worker.join().unwrap();
    assert_fault(&response, "fs_error");
}

#[test]
fn stat_real_host_fragmented_stream_cannot_extend_five_second_rpc_deadline() {
    let (listener, native) = bridge();
    let worker = thread::spawn(move || {
        let (mut stream, _) = receive(&listener);
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            if stream.write_all(b" ").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        assert!(matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
    });
    let began = Instant::now();
    let (_, response) = invoke(
        &format!(
            "return ctx.fs.stat({});",
            serde_json::to_string(PATH).unwrap()
        ),
        Some(native),
    );
    assert!(
        began.elapsed() < Duration::from_secs(8),
        "零碎数据不能延长只读 RPC 总时限"
    );
    worker.join().unwrap();
    assert_fault(&response, "timeout");
}
