# Neovim integration

Most users receive type diagnostics automatically through `m1-lsp`. This plugin is for standalone use (e.g. CI pipelines) or for users who want to run the type checker directly without the language server.

## Requirements

- [nvim-lint](https://github.com/mfussenegger/nvim-lint)
- Rust toolchain (to build the binary via `cargo build --release`)

## Installation (lazy.nvim)

```lua
{
  'C-Nucifora/m1-typecheck',
  build = 'cargo build --release',
  dependencies = { 'mfussenegger/nvim-lint' },
  config = function()
    require('m1_typecheck').setup({})
  end,
}
```

## Options

Pass a table to `setup()` to override defaults:

- `auto_lint` (boolean, default `true`): register `BufWritePost` and `InsertLeave` autocmds that call `lint.try_lint()` on `.m1scr` files. Set to `false` to manage linting triggers yourself.
- `linter` (table): merged (via `vim.tbl_deep_extend`) into the nvim-lint linter definition. Use this to override `cmd`, `args`, or any other field.

## Avoiding duplicate diagnostics

When `m1-lsp` is also active it emits the same type diagnostics (T-code rules). To avoid seeing them twice either:

- Disable T-code rules in `m1-lsp` (set `typecheck = false` in the server settings), or
- Use only this plugin and do not attach `m1-lsp` for type diagnostics.
