use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{exit, Command};
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;

use serde_json::Value;

macro_rules! log_err {
    ($($arg:tt)*) => {
        let _ = std::io::stderr().write_all(format!("{}\n", format_args!($($arg)*)).as_bytes());
    };
}

static DXC_PATH: OnceLock<PathBuf> = OnceLock::new();
static MAX_SHADER_MODELS: OnceLock<HashMap<ShaderTarget, String>> = OnceLock::new();

const SERVER_NAME: &str = env!("CARGO_PKG_NAME");
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

struct CompileRequest {
    path: PathBuf,
    // TODO: Check if we need to pass more info here based on LSP commands
}

#[derive(Debug, Hash, Eq, PartialEq, Copy, Clone)]
enum ShaderTarget {
    Vertex,
    Pixel,
    Compute,
    Library,
}

struct CompileParams {
    target: ShaderTarget,
    entry_point: Option<String>,
}

fn spawn_worker() -> Sender<CompileRequest> {
    let (tx, rx) = mpsc::channel::<CompileRequest>();

    thread::spawn(move || {
        // recv() blocks until a request arrives. Will only stop looping after all Senders are closed
        while let Ok(req) = rx.recv() {
            log_err!("[hlsl-ls] compiling {:?} using dxc found at {DXC_PATH:?}", req.path);
            let params = detect_compile_params(&req.path);

            let sm = MAX_SHADER_MODELS.get()
                .and_then(|m| m.get(&params.target))
                .map(|s| s.as_str())
                .unwrap_or("6_0");

            let target = match params.target {
                ShaderTarget::Vertex => format!("vs_{sm}"),
                ShaderTarget::Pixel => format!("ps_{sm}"),
                ShaderTarget::Compute => format!("cs_{sm}"),
                ShaderTarget::Library => format!("lib_{sm}"),
            };

            let mut cmd_line: Vec<String> = vec!["-T".to_owned(), target];

            if let Some(entry) = params.entry_point {
                if !entry.is_empty() {
                    cmd_line.push("-E".to_owned());
                    cmd_line.push(entry);
                }
            }

            // TODO: invoke dxc, publish diagnostics back over stdout
            log_err!("[hlsl-ls] command line params: {:?}", cmd_line);
        }
        log_err!("[hlsl-ls] worker shutting down");
    });

    tx
}

fn detect_max_shader_models(dxc_path: &Path) -> HashMap<ShaderTarget, String> {
    let mut result = HashMap::new();

    let output = match Command::new(dxc_path).arg("-help").output() {
        Ok(o) => o,
        Err(e) => {
            log_err!("[hlsl-ls] failed to run `{} -help`: {e}", dxc_path.display());
            return result;
        }
    };
    let help = String::from_utf8_lossy(&output.stdout);

    let prefixes = [
        ("vs_6_", ShaderTarget::Vertex),
        ("ps_6_", ShaderTarget::Pixel),
        ("cs_6_", ShaderTarget::Compute),
        ("lib_6_", ShaderTarget::Library),
    ];

    let mut maxes: HashMap<ShaderTarget, u32> = HashMap::new();
    for word in help.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        for (prefix, stage) in &prefixes {
            if let Some(rest) = word.strip_prefix(prefix) {
                if let Ok(n) = rest.parse::<u32>() {
                    let slot = maxes.entry(*stage).or_insert(0);
                    *slot = (*slot).max(n);
                }
            }
        }
    }

    for (stage, n) in maxes {
        result.insert(stage, format!("6_{n}"));
    }

    log_err!("[hlsl-ls] Detected best possible shader models -> {:?}", result);
    result
}

fn detect_compile_params(shader_path: &Path) -> CompileParams {
    // TODO: Will need to parse file to capture entry point of shader and target at least
    //  Start with custom comment header, fallback to heuristic parsing if not available
    let shader_file = match File::open(shader_path) {
        Ok(f) => f,
        Err(_) => {
            log_err!("[hlsl-ls] Error opening file: {}; default to library shader target", shader_path.display());
            return CompileParams { target: ShaderTarget::Library, entry_point: None };
        },
    };

    let reader = BufReader::new(shader_file);
    let mut target = ShaderTarget::Library;
    let mut entry_point = None;

    for file_line in reader.lines() {
        let line = match file_line {
            Ok(l) => l,
            Err(_) => {
                log_err!("[hlsl-ls] Error reading file: {}", shader_path.display());
                break;
            },
        };

        if let Some(_header) = line.strip_prefix("// hlsl-ls:") {
            // Found owr own header format; parse, set values and break
            break;
        }

        // Keep searching for possible heuristics to detect shader target / entry point

        target = ShaderTarget::Library;
        entry_point = None;
    }

    return CompileParams { target, entry_point };
}

fn main() {
    log_err!("[hlsl-ls] Starting language server ...");

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
                log_err!("[hlsl-ls] malformed JSON-RPC body: {e}");
                continue;
            }
        };

        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id");

        log_err!("[hlsl-ls] recv method={method:?} id={id:?}");

        match method {
            "initialize" => { init_handler(id.unwrap(), msg.get("params").unwrap()); }
            "initialized" => { initialized_handler(); }
            "textDocument/didOpen" => { did_open_handler(&work_tx, msg.get("params").unwrap()); }
            "textDocument/didSave" => { did_save_handler(&work_tx, msg.get("params").unwrap()); }
            "shutdown" => { shutdown_handler(id.unwrap()); }
            "exit" => { exit_handler(); }
            name => {
                log_err!("[hlsl-ls] Cannot handle method name {name}!");
                continue;
            }
        }
    }
}

fn init_handler(id: &Value, obj: &Value) {
    let str_params = serde_json::to_string(obj).unwrap();
    match serde_json::from_str::<lsp_types::InitializeParams>(&str_params) {
        Ok(params) => {
            let dxc_path = params.initialization_options
                .as_ref()
                .and_then(|opts| opts.get("dxc_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("dxc"));

            log_err!("[hlsl-ls] dxc path: {}", dxc_path.display());

            let models = detect_max_shader_models(&dxc_path);
            MAX_SHADER_MODELS.set(models).ok();

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
            let path_str = p.text_document.uri.path().as_str();
            let path_str = path_str.strip_prefix('/').unwrap_or(path_str); // Windows drive letter fix
            let path = PathBuf::from(path_str);
            work_tx.send(CompileRequest { path }).unwrap();
        },
        Err(_) => { panic!("[hlsl-ls] Could not parse textDocument/didOpen parameters") },
    }
}

fn did_save_handler(work_tx: &Sender<CompileRequest>, params: &Value) {
    let str_params = serde_json::to_string(params).unwrap();
    match serde_json::from_str::<lsp_types::DidSaveTextDocumentParams>(&str_params) {
        Ok(p) => {
            let path_str = p.text_document.uri.path().as_str();
            let path_str = path_str.strip_prefix('/').unwrap_or(path_str); // Windows drive letter fix
            let path = PathBuf::from(path_str);
            work_tx.send(CompileRequest { path }).unwrap();
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

fn write_to_client(msg: &impl serde::Serialize) {
    let body = serde_json::to_string(msg).unwrap();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write!(stdout, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
    stdout.flush().unwrap();
}
