local M = {}

local function parse_line(line)
  local lnum, col, sev, code, msg =
    line:match('^[^:]+:(%d+):(%d+): (%a+)%[([A-Z0-9]+)%]: (.+)$')
  if not lnum then return nil end
  local severity
  local s = sev:lower()
  if s == 'error' then
    severity = vim.diagnostic.severity.ERROR
  elseif s == 'warning' then
    severity = vim.diagnostic.severity.WARN
  elseif s == 'hint' then
    severity = vim.diagnostic.severity.HINT
  else
    severity = vim.diagnostic.severity.INFO
  end
  return {
    lnum     = tonumber(lnum) - 1,
    col      = tonumber(col) - 1,
    severity = severity,
    message  = string.format('[%s] %s', code, msg),
    source   = 'm1-typecheck',
  }
end

function M.setup(opts)
  opts = opts or {}
  local plugin_dir = vim.fn.fnamemodify(debug.getinfo(1, 'S').source:sub(2), ':h:h:h')
  local bin = plugin_dir .. '/target/release/m1-typecheck'

  local ok, lint = pcall(require, 'lint')
  if not ok then
    vim.notify('m1_typecheck: nvim-lint not found', vim.log.levels.WARN)
    return
  end

  lint.linters.m1_typecheck = vim.tbl_deep_extend('force', {
    name    = 'm1-typecheck',
    cmd     = bin,
    stdin   = false,
    args    = function()
      local project = vim.fn.findfile('Project.m1prj', '.;')
      local args = {}
      if project ~= '' then
        args[#args + 1] = '--project'
        args[#args + 1] = vim.fn.fnamemodify(project, ':p')
      end
      return args
    end,
    stream  = 'stdout',
    ignore_exitcode = true,
    parser  = function(output, _)
      local diags = {}
      for _, line in ipairs(vim.split(output, '\n', { plain = true })) do
        local d = parse_line(line)
        if d then diags[#diags + 1] = d end
      end
      return diags
    end,
  }, opts.linter or {})

  lint.linters_by_ft = vim.tbl_deep_extend('force',
    lint.linters_by_ft or {},
    { m1scr = vim.list_extend(
        lint.linters_by_ft and lint.linters_by_ft.m1scr or {},
        { 'm1_typecheck' }
      )
    }
  )

  if opts.auto_lint ~= false then
    vim.api.nvim_create_autocmd({ 'BufWritePost', 'InsertLeave' }, {
      pattern = '*.m1scr',
      callback = function() lint.try_lint() end,
    })
  end
end

return M
