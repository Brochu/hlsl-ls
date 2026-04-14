# hlsl-ls

A minimal HLSL language server that provides real-time diagnostics from [DXC (DirectX Shader Compiler)](https://github.com/microsoft/DirectXShaderCompiler) directly in your text editor.

The server compiles HLSL shaders on save and reports errors and warnings as LSP diagnostics, giving you immediate feedback without leaving your editor.

## Features

- Compile-on-save diagnostics via DXC
- Minimal dependencies, lightweight and fast
- Configurable DXC path (falls back to `dxc` on PATH)

## Building

```
cargo build --release
```

The binary will be at `target/release/hlsl-ls.exe`.

## Neovim Setup

Add the following to your Neovim config (e.g. `init.lua`):

```lua
vim.lsp.config('hlsl_ls', {
    cmd = { "/path/to/hlsl-ls.exe" },
    filetypes = { "hlsl" },
    root_markers = { ".git" },
    init_options = {
        dxc_path = "/path/to/dxc.exe",  -- optional, falls back to "dxc" on PATH
    },
})

vim.lsp.enable({ 'hlsl_ls' })
```

If Neovim does not detect `.hlsl` files automatically, add filetype detection:

```lua
vim.filetype.add({ extension = { hlsl = "hlsl", hlsli = "hlsl" } })
```

## Status

Under active development. Currently supports the LSP initialization handshake and `textDocument/didSave` notifications. Diagnostic reporting from DXC is in progress.
