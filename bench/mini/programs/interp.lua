-- interp: a small interpreter written in Mini. A scanner (string.byte /
-- string.sub) and recursive-descent parser turn a tiny imperative language
-- into an AST, a compiler flattens it to stack-machine code, and a dispatch
-- loop runs it (if/elseif chain on opcodes, stack and variable arrays).
-- Original.
local N = 15000          -- argument passed to the interpreted program
local PARSE_REPEAT = 200

local SRC = table.concat({
  "# count primes below n by trial division",
  "primes = 0; i = 2;",
  "while i < n {",
  "  d = 2; isp = 1;",
  "  while d * d < i + 1 {",
  "    if i % d == 0 { isp = 0; d = i; } else { d = d + 1; }",
  "  }",
  "  primes = primes + isp;",
  "  i = i + 1;",
  "}",
  "print primes;",
  "# total collatz steps for 1..n",
  "steps = 0; k = 1; longest = 0; arg = 0;",
  "while k < n + 1 {",
  "  x = k; s = 0;",
  "  while x > 1 {",
  "    if x % 2 == 0 { x = x / 2; } else { x = 3 * x + 1; }",
  "    s = s + 1;",
  "  }",
  "  steps = steps + s;",
  "  if s > longest { longest = s; arg = k; }",
  "  k = k + 1;",
  "}",
  "print steps; print longest; print arg;",
  "# a nested arithmetic loop",
  "acc = 0; a = 0;",
  "while a < 300 {",
  "  b = 0;",
  "  while b < 300 { acc = (acc * 7 + a * b - (a - b) % 13) % 100003; b = b + 1; }",
  "  a = a + 1;",
  "}",
  "print acc;",
}, "\n")

---------------------------------------------------------------- scanner
local function is_digit(c) return c >= 48 and c <= 57 end
local function is_alpha(c)
  return (c >= 97 and c <= 122) or (c >= 65 and c <= 90) or c == 95
end

local function scan(src)
  local toks = {}
  local i, n = 1, #src
  while i <= n do
    local c = string.byte(src, i)
    if c == 32 or c == 10 or c == 9 then
      i = i + 1
    elseif c == 35 then -- '#' comment to end of line
      while i <= n and string.byte(src, i) ~= 10 do i = i + 1 end
    elseif is_digit(c) then
      local v = 0
      while i <= n and is_digit(string.byte(src, i)) do
        v = v * 10 + (string.byte(src, i) - 48)
        i = i + 1
      end
      toks[#toks + 1] = { k = "num", v = v }
    elseif is_alpha(c) then
      local j = i
      while j <= n do
        local d = string.byte(src, j)
        if is_alpha(d) or is_digit(d) then j = j + 1 else break end
      end
      local word = string.sub(src, i, j - 1)
      if word == "while" or word == "if" or word == "else" or word == "print" then
        toks[#toks + 1] = { k = word }
      else
        toks[#toks + 1] = { k = "id", v = word }
      end
      i = j
    elseif c == 61 and string.byte(src, i + 1) == 61 then
      toks[#toks + 1] = { k = "==" }
      i = i + 2
    else
      toks[#toks + 1] = { k = string.char(c) }
      i = i + 1
    end
  end
  toks[#toks + 1] = { k = "eof" }
  return toks
end

---------------------------------------------------------------- parser
local function parse(toks)
  local pos = 1
  local function peek() return toks[pos].k end
  local function next_tok() local t = toks[pos]; pos = pos + 1; return t end
  local function expect(k)
    local t = next_tok()
    if t.k ~= k then error("expected " .. k .. " got " .. t.k) end
    return t
  end

  local expr

  local function primary()
    local t = next_tok()
    if t.k == "num" then return { op = "num", v = t.v }
    elseif t.k == "id" then return { op = "var", name = t.v }
    elseif t.k == "(" then
      local e = expr()
      expect(")")
      return e
    end
    error("unexpected token " .. t.k)
  end

  local function mul()
    local e = primary()
    while peek() == "*" or peek() == "/" or peek() == "%" do
      local o = next_tok().k
      e = { op = o, l = e, r = primary() }
    end
    return e
  end

  local function add()
    local e = mul()
    while peek() == "+" or peek() == "-" do
      local o = next_tok().k
      e = { op = o, l = e, r = mul() }
    end
    return e
  end

  expr = function()
    local e = add()
    local k = peek()
    if k == "<" or k == ">" or k == "==" then
      next_tok()
      e = { op = k, l = e, r = add() }
    end
    return e
  end

  local block

  local function statement()
    local k = peek()
    if k == "while" then
      next_tok()
      local c = expr()
      return { op = "while", cond = c, body = block() }
    elseif k == "if" then
      next_tok()
      local c = expr()
      local th = block()
      local el = nil
      if peek() == "else" then next_tok(); el = block() end
      return { op = "if", cond = c, th = th, el = el }
    elseif k == "print" then
      next_tok()
      local e = expr()
      expect(";")
      return { op = "print", e = e }
    end
    local name = expect("id").v
    expect("=")
    local e = expr()
    expect(";")
    return { op = "set", name = name, e = e }
  end

  block = function()
    expect("{")
    local stmts = {}
    while peek() ~= "}" do stmts[#stmts + 1] = statement() end
    expect("}")
    return stmts
  end

  local prog = {}
  while peek() ~= "eof" do prog[#prog + 1] = statement() end
  return prog
end

---------------------------------------------------------------- compiler
-- opcodes
local PUSH, LOAD, STORE, ADD, SUB, MUL, DIV, MOD = 1, 2, 3, 4, 5, 6, 7, 8
local LT, GT, EQ, JMP, JZ, PRINT, HALT = 9, 10, 11, 12, 13, 14, 15
local BINOPS = { ["+"] = ADD, ["-"] = SUB, ["*"] = MUL, ["/"] = DIV,
                 ["%"] = MOD, ["<"] = LT, [">"] = GT, ["=="] = EQ }

local function compile(prog)
  local code, args = {}, {}
  local slots, nslots = {}, 0
  local function slot(name)
    local s = slots[name]
    if not s then nslots = nslots + 1; s = nslots; slots[name] = s end
    return s
  end
  local function emit(op, a)
    code[#code + 1] = op
    args[#code] = a or 0
    return #code
  end

  local function cexpr(e)
    local op = e.op
    if op == "num" then emit(PUSH, e.v)
    elseif op == "var" then emit(LOAD, slot(e.name))
    else
      cexpr(e.l); cexpr(e.r)
      emit(BINOPS[op])
    end
  end

  local cblock
  local function cstmt(s)
    local op = s.op
    if op == "set" then
      cexpr(s.e); emit(STORE, slot(s.name))
    elseif op == "print" then
      cexpr(s.e); emit(PRINT)
    elseif op == "while" then
      local top = #code + 1
      cexpr(s.cond)
      local jz = emit(JZ)
      cblock(s.body)
      emit(JMP, top)
      args[jz] = #code + 1
    elseif op == "if" then
      cexpr(s.cond)
      local jz = emit(JZ)
      cblock(s.th)
      if s.el then
        local j = emit(JMP)
        args[jz] = #code + 1
        cblock(s.el)
        args[j] = #code + 1
      else
        args[jz] = #code + 1
      end
    end
  end
  cblock = function(b) for i = 1, #b do cstmt(b[i]) end end

  cblock(prog)
  emit(HALT)
  return code, args, slots, nslots
end

---------------------------------------------------------------- VM
local function run(code, args, vars, out)
  local stack, sp, pc = {}, 0, 1
  local steps = 0
  while true do
    local op = code[pc]
    local a = args[pc]
    pc = pc + 1
    steps = steps + 1
    if op == LOAD then
      sp = sp + 1; stack[sp] = vars[a]
    elseif op == PUSH then
      sp = sp + 1; stack[sp] = a
    elseif op == STORE then
      vars[a] = stack[sp]; sp = sp - 1
    elseif op == JZ then
      if stack[sp] == 0 then pc = a end
      sp = sp - 1
    elseif op == JMP then
      pc = a
    elseif op == ADD then
      sp = sp - 1; stack[sp] = stack[sp] + stack[sp + 1]
    elseif op == SUB then
      sp = sp - 1; stack[sp] = stack[sp] - stack[sp + 1]
    elseif op == MUL then
      sp = sp - 1; stack[sp] = stack[sp] * stack[sp + 1]
    elseif op == DIV then
      sp = sp - 1; stack[sp] = math.floor(stack[sp] / stack[sp + 1])
    elseif op == MOD then
      sp = sp - 1; stack[sp] = stack[sp] % stack[sp + 1]
    elseif op == LT then
      sp = sp - 1; if stack[sp] < stack[sp + 1] then stack[sp] = 1 else stack[sp] = 0 end
    elseif op == GT then
      sp = sp - 1; if stack[sp] > stack[sp + 1] then stack[sp] = 1 else stack[sp] = 0 end
    elseif op == EQ then
      sp = sp - 1; if stack[sp] == stack[sp + 1] then stack[sp] = 1 else stack[sp] = 0 end
    elseif op == PRINT then
      out[#out + 1] = stack[sp]; sp = sp - 1
    elseif op == HALT then
      return steps
    else
      error("bad opcode")
    end
  end
end

---------------------------------------------------------------- main
local prog
local ntok = 0
for r = 1, PARSE_REPEAT do
  local toks = scan(SRC)
  ntok = #toks
  prog = parse(toks)
end
local code, args, slots, nslots = compile(prog)
local vars = {}
for i = 1, nslots do vars[i] = 0 end
vars[slots["n"]] = N
local out = {}
local steps = run(code, args, vars, out)

print(string.format("tokens %d code %d vars %d", ntok, #code, nslots))
for i = 1, #out do print(string.format("out %d", out[i])) end
print(string.format("steps %d", steps))
