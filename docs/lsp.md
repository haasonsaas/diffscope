# LSP Setup and Troubleshooting

This guide covers installation options, recommended commands, and common pitfalls when using the LSP-backed symbol indexer.

## Auto-detection

If `symbol_index_provider: lsp` and `symbol_index_lsp_command` is not set, diffscope will try to auto-detect a server by:
- scanning repository file extensions (honoring ignore rules), and
- selecting the first available server that matches your configured `symbol_index_lsp_languages`.

If it picks the wrong server, set `symbol_index_lsp_command` explicitly.

## CLI Overrides

- Force a command for this run: `diffscope --lsp-command "typescript-language-server --stdio" smart-review --diff diff.patch`
- Preflight your setup: `diffscope lsp-check` (prints the detected command, whether it is available on PATH, and any unmapped extensions)
- Ready-to-copy configs live in `examples/lsp/`.

## Package Manager Matrix (brew/apt/choco)

Use these when you prefer OS-level packages. If a cell says "manual", use the language-specific install section below.

| Server | macOS (brew) | Ubuntu/Debian (apt) | Windows (choco) |
| --- | --- | --- | --- |
| rust-analyzer | `brew install rust-analyzer` | `rustup component add rust-analyzer` | `rustup component add rust-analyzer` |
| clangd | `brew install llvm` | `sudo apt-get install clangd-12` | `choco install llvm` |
| kotlin-lsp | `brew install JetBrains/utils/kotlin-lsp` | manual (zip) | manual (zip) |
| Eclipse JDT LS | manual (zip) | manual (zip) | manual (zip) |

## Language-specific Installers (cross-platform)

Set `symbol_index_lsp_command` to the command shown after installation.

- Rust (rust-analyzer)
  - Install: `rustup component add rust-analyzer`
  - Optional: `rustup component add rust-src`
  - Command: `rust-analyzer`
- TypeScript / JavaScript (typescript-language-server)
  - Install: `npm install -g typescript-language-server typescript`
  - Command: `typescript-language-server --stdio`
- Python (python-lsp-server / pylsp)
  - Install: `pip install "python-lsp-server[all]"` (or `python-lsp-server`)
  - Command: `pylsp`
- Go (gopls)
  - Install: `go install golang.org/x/tools/gopls@latest`
  - Command: `gopls`
- C# (csharp-ls)
  - Install: `dotnet tool install --global csharp-ls`
  - Command: `csharp-ls`
- Ruby (solargraph)
  - Install: `gem install solargraph`
  - Command: `solargraph stdio`
- PHP (Phpactor)
  - Install (PHAR): `curl -Lo phpactor.phar https://github.com/phpactor/phpactor/releases/latest/download/phpactor.phar`
  - Then: `chmod a+x phpactor.phar` and `mv phpactor.phar ~/.local/bin/phpactor`
  - Command: `phpactor language-server`

## Manual Installers

### Kotlin LSP

If you didn't use Homebrew:
1. Download the standalone zip from the Kotlin LSP Releases page.
2. `chmod +x $KOTLIN_LSP_DIR/kotlin-lsp.sh`
3. Symlink it: `ln -s $KOTLIN_LSP_DIR/kotlin-lsp.sh $HOME/.local/bin/kotlin-lsp`

### Eclipse JDT LS

Download and extract a milestone or snapshot build. You can run the wrapper:

```
bin/jdtls -configuration /path/to/config -data /path/to/workspace
```

## Troubleshooting

- Auto-detect chose the wrong server: set `symbol_index_lsp_command` explicitly.
- LSP binary not found: check PATH or use an absolute path in `symbol_index_lsp_command`.
- Server expects stdio: use the `--stdio` form when required (e.g., TypeScript, Solargraph).
- JDT LS needs a unique `-data` directory per workspace; configure a stable path.
- Use `diffscope lsp-check` to see unmapped extensions and missing binaries.
