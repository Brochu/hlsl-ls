use std::io::{self, BufRead, Read, Write, stderr};
use std::path::PathBuf;
use std::process::exit;
use std::sync::mpsc::{self, Sender};
use std::thread;

struct CompileRequest {
    path: PathBuf,
    // TODO: Check if we need to pass more info here based on LSP commands
}

fn spawn_worker() -> Sender<CompileRequest> {
    let (tx, rx) = mpsc::channel::<CompileRequest>();

    thread::spawn(move || {
        // recv() blocks until a request arrives.
        // It returns Err only once every Sender has been dropped,
        // which is our natural shutdown signal.
        while let Ok(req) = rx.recv() {
            writeln!(stderr(), "[hlsl-ls] compiling {:?}", req.path).unwrap();
            // TODO: Will need to parse file to capture entry point of shader and target at least
            //  Start with custom comment header, fallback to heuristic parsing if not available
            // TODO: invoke dxc, publish diagnostics back over stdout
        }
        writeln!(stderr(), "[hlsl-ls] worker shutting down").unwrap();
    });

    tx
}

fn main() {
    writeln!(stderr(), "[hlsl-ls] Starting language server ...").unwrap();

    let _work_tx = spawn_worker();

    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    loop {
        let mut cmd_len: usize = 0;
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.read_line(&mut line) {
                Ok(0) => exit(0), // stdin closed
                Ok(_) => {
                    if line == "\r\n" { break; }
                    if let Some(v) = line.strip_prefix("Content-Length:") {
                        cmd_len = v.trim().parse().unwrap();
                    }
                }
                Err(_) => panic!("Could not read from stdin!"),
            }
        }

        let mut cmd_buf = vec![0u8; cmd_len];
        stdin.read_exact(&mut cmd_buf).expect("short read on body");

        let msg: serde_json::Value = match serde_json::from_slice(&cmd_buf) {
            Ok(v) => v,
            Err(e) => {
                writeln!(stderr(), "[hlsl-ls] malformed JSON-RPC body: {e}").unwrap();
                continue;
            }
        };

        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id");

        writeln!(stderr(), "[hlsl-ls] recv method={method:?} id={id:?}").unwrap();

        match method {
            "initialize" => {
                // TODO: reply with ServerCapabilities
            }
            "initialized" => {
                // Notification, no reply needed
            }
            "textDocument/didSave" => {
                // TODO: extract path and push a CompileRequest onto work_tx
            }
            "shutdown" => {
                // TODO: reply with null result
            }
            "exit" => {
                exit(0);
            }
            name => {
                writeln!(stderr(), "[hlsl-ls] Cannot handle method name {name}!").unwrap();
                continue;
            }
        }
    }
}
