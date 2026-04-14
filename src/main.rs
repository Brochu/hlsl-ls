use std::io::{self, BufRead, Read, Write, stderr};
use std::path::PathBuf;
use std::process::exit;
use std::str::FromStr;
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;

use serde_json::{ Value };

static DXC_PATH: OnceLock<PathBuf> = OnceLock::new();

const SERVER_NAME: &str = env!("CARGO_PKG_NAME");
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

struct CompileRequest {
    path: PathBuf,
    // TODO: Check if we need to pass more info here based on LSP commands
}

fn spawn_worker() -> Sender<CompileRequest> {
    let (tx, rx) = mpsc::channel::<CompileRequest>();

    thread::spawn(move || {
        // recv() blocks until a request arrives. Will only stop looping after all Senders are closed
        while let Ok(req) = rx.recv() {
            writeln!(stderr(), "[hlsl-ls] compiling {:?} using dxc found at {DXC_PATH:?}", req.path).unwrap();
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

    let work_tx = spawn_worker();

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
                Err(_) => panic!("[hlsl-ls] Could not read from stdin!"),
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
            "initialize" => { init_handler(id.unwrap(), msg.get("params").unwrap()); }
            "initialized" => { initialized_handler(); }
            "textDocument/didOpen" => { did_open_handler(&work_tx, msg.get("params").unwrap()); }
            "textDocument/didSave" => { did_save_handler(&work_tx, msg.get("params").unwrap()); }
            "shutdown" => { shutdown_handler(id.unwrap()); }
            "exit" => { exit_handler(); }
            name => {
                writeln!(stderr(), "[hlsl-ls] Cannot handle method name {name}!").unwrap();
                continue;
            }
        }
    }
}

fn write_to_client(msg: &impl serde::Serialize) {
    let body = serde_json::to_string(msg).unwrap();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write!(stdout, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
    stdout.flush().unwrap();
}

fn init_handler(id: &Value, obj: &Value) {
    let str_params = serde_json::to_string(obj).unwrap();
    match serde_json::from_str::<lsp_types::InitializeParams>(&str_params) {
        Ok(params) => {
            let dxc_path = params.initialization_options
                .as_ref()
                .and_then(|opts| opts.get("dxc_path"))
                .and_then(|v| v.as_str())
                .and_then(|s| Some(s.trim()))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("dxc"));

            writeln!(stderr(), "[hlsl-ls] dxc path: {}", dxc_path.display()).unwrap();
            DXC_PATH.set(dxc_path).expect("[hlsl-ls] init_handler called twice");

            let capabilities = lsp_types::ServerCapabilities {
                text_document_sync: Some(lsp_types::TextDocumentSyncCapability::Options(lsp_types::TextDocumentSyncOptions {
                    save: Some(lsp_types::TextDocumentSyncSaveOptions::Supported(true)),
                    open_close: Some(true),
                    ..Default::default()
                    })),
                ..Default::default()
            };

            let result = lsp_types::InitializeResult {
                capabilities,
                server_info: Some(lsp_types::ServerInfo {
                    name: SERVER_NAME.to_owned(),
                    version: Some(SERVER_VERSION.to_owned()),
                }),
            };
            write_to_client(&serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            }));
        },
        Err(_) => { panic!("[hlsl-ls] Could not parse initialization parameters") },
    }
}

fn initialized_handler() {
    // Notification, no reply needed
}

fn did_open_handler(work_tx: &Sender<CompileRequest>, params: &Value) {
    let str_params = serde_json::to_string(params).unwrap();
    match serde_json::from_str::<lsp_types::DidOpenTextDocumentParams>(&str_params) {
        Ok(p) => {
            match PathBuf::from_str(p.text_document.uri.as_str()) {
                Ok(path) => { work_tx.send(CompileRequest { path }).unwrap(); }
                _ => (),
            }
        },
        Err(_) => { panic!("[hlsl-ls] Could not parse textDocument/didOpen parameters") },
    }
}

fn did_save_handler(work_tx: &Sender<CompileRequest>, params: &Value) {
    let str_params = serde_json::to_string(params).unwrap();
    match serde_json::from_str::<lsp_types::DidSaveTextDocumentParams>(&str_params) {
        Ok(p) => {
            match PathBuf::from_str(p.text_document.uri.as_str()) {
                Ok(path) => { work_tx.send(CompileRequest { path }).unwrap(); }
                _ => (),
            }
        },
        Err(_) => { panic!("[hlsl-ls] Could not parse textDocument/didSave parameters") },
    }
}

fn shutdown_handler(id: &Value) {
    write_to_client(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": null
    }));
}

fn exit_handler() {
    exit(0);
}
