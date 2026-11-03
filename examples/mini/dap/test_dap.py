#!/usr/bin/env python3
"""Drive mini-dap (examples/mini/dap/mini_dap.c) through a scripted Debug
Adapter Protocol session on examples/mini/tests/dap/session.mini and
print the transcript: each request, then every message the adapter sent
until the response (or the event waited for), without sequence numbers,
consecutive output events joined, and the script's directory as $DIR.

    test_dap.py ADAPTER [SESSION.mini] > transcript

tests/golden.rs compares the transcript with session.out.
"""
import json, os, signal, subprocess, sys, time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))


class Session:
    def __init__(self, adapter, program):
        self.p = subprocess.Popen([adapter], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.seq = 1
        self.dir = os.path.dirname(program)
        self.pending = []  # output events not printed yet

    def send(self, command, args=None, show=True):
        msg = {"seq": self.seq, "type": "request", "command": command}
        if args is not None:
            msg["arguments"] = args
        self.seq += 1
        data = json.dumps(msg).encode()
        self.p.stdin.write(b"Content-Length: %d\r\n\r\n" % len(data) + data)
        self.p.stdin.flush()
        if show:
            print("->", command, self.norm(args) if args is not None else "")
        return msg["seq"]

    def read(self):
        n = None
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise SystemExit("adapter closed its output")
            line = line.strip()
            if not line:
                break
            k, v = line.split(b":", 1)
            if k.lower() == b"content-length":
                n = int(v)
        return json.loads(self.p.stdout.read(n))

    def norm(self, obj):
        s = json.dumps(obj, sort_keys=True)
        return s.replace(self.dir, "$DIR")

    def show(self, m):
        if m["type"] == "event" and m["event"] == "output":
            b = m["body"]
            if self.pending and self.pending[-1]["category"] == b["category"]:
                self.pending[-1]["output"] += b["output"]
            else:
                self.pending.append(dict(b))
            return
        self.flush_output()
        if m["type"] == "response":
            ok = "ok" if m["success"] else "FAILED"
            extra = (" " + json.dumps(m["message"])) if "message" in m else ""
            print("<- response", m["command"], ok + extra, self.norm(m["body"]) if "body" in m else "")
        else:
            print("<- event", m["event"], self.norm(m["body"]) if "body" in m else "")

    def flush_output(self):
        for b in self.pending:
            print("<- event output", self.norm(b))
        self.pending = []

    def request(self, command, args=None):
        """Send, and print everything up to the response, which is returned."""
        seq = self.send(command, args)
        while True:
            m = self.read()
            self.show(m)
            if m["type"] == "response" and m["request_seq"] == seq:
                return m

    def wait(self, event):
        while True:
            m = self.read()
            self.show(m)
            if m["type"] == "event" and m["event"] == event:
                return m


def main():
    adapter = sys.argv[1]
    program = os.path.abspath(sys.argv[2] if len(sys.argv) > 2 else os.path.join(ROOT, "examples/mini/tests/dap/session.mini"))
    signal.alarm(60)
    s = Session(adapter, program)
    src = {"path": program}
    s.request("initialize", {"adapterID": "mini", "linesStartAt1": True, "columnsStartAt1": True})
    s.request("launch", {"program": program, "stopOnEntry": True})
    s.wait("initialized")
    # Line 3 when n == 4; line 9 at its third hit; line 5 is blank (moves
    # to 6); line 40 has no code; a bad hit condition.
    s.request("setBreakpoints", {"source": src, "breakpoints": [
        {"line": 3, "condition": "n == 4"}, {"line": 9, "hitCondition": "3"},
        {"line": 5}, {"line": 40}, {"line": 2, "hitCondition": "lots"}]})
    s.request("setExceptionBreakpoints", {"filters": ["error"]})
    s.request("configurationDone")
    s.wait("stopped")                       # entry
    s.request("threads")
    s.request("stackTrace", {"threadId": 1})
    s.request("continue", {"threadId": 1})
    s.wait("stopped")                       # line 6 (set at 5)
    s.request("continue", {"threadId": 1})
    s.wait("stopped")                       # line 9, i == 3
    s.request("stackTrace", {"threadId": 1})
    sc = s.request("scopes", {"frameId": 1})["body"]["scopes"]
    loc = s.request("variables", {"variablesReference": sc[0]["variablesReference"]})["body"]["variables"]
    s.request("variables", {"variablesReference": sc[1]["variablesReference"]})
    config = [v for v in loc if v["name"] == "config"][0]
    sub = s.request("variables", {"variablesReference": config["variablesReference"]})["body"]["variables"]
    sizes = [v for v in sub if v["name"] == "sizes"][0]
    s.request("variables", {"variablesReference": sizes["variablesReference"], "start": 1, "count": 2})
    s.request("evaluate", {"expression": "total * 10 + i", "frameId": 1, "context": "watch"})
    # fib(5) passes line 3 with n == 4, but breakpoints are off while it runs.
    s.request("evaluate", {"expression": "fib(5)", "frameId": 1, "context": "repl"})
    s.request("evaluate", {"expression": "config.nested", "frameId": 1, "context": "hover"})
    s.request("evaluate", {"expression": "config.nope.x", "frameId": 1, "context": "watch"})
    s.request("continue", {"threadId": 1})
    s.wait("stopped")                       # line 3, n == 4
    s.request("stackTrace", {"threadId": 1})
    s.request("next", {"threadId": 1})     # fib(4) returns: the caller
    s.wait("stopped")
    s.request("stackTrace", {"threadId": 1, "levels": 1})
    s.request("setBreakpoints", {"source": src, "breakpoints": []})
    for _ in range(3):                      # the loop's end, line 9, into fib
        s.request("stepIn", {"threadId": 1})
        s.wait("stopped")
        s.request("stackTrace", {"threadId": 1, "levels": 1})
    s.request("stepOut", {"threadId": 1})
    s.wait("stopped")
    s.request("stackTrace", {"threadId": 1})
    s.request("continue", {"threadId": 1})
    # Running, in the loop on line 16 that only `done` ends. A breakpoint
    # set now (the adapter stops the program for it, unseen), then a pause.
    time.sleep(0.3)
    s.request("setBreakpoints", {"source": src, "breakpoints": [{"line": 17}]})
    time.sleep(0.1)
    s.request("pause", {"threadId": 1})    # the stopped event comes first
    s.request("stackTrace", {"threadId": 1})
    s.request("evaluate", {"expression": "n > 0", "frameId": 1, "context": "watch"})
    s.request("evaluate", {"expression": "done = true", "frameId": 1, "context": "repl"})
    s.request("continue", {"threadId": 1})
    s.wait("stopped")                       # line 17
    s.request("continue", {"threadId": 1})
    s.wait("stopped")                       # the error, in check
    s.request("stackTrace", {"threadId": 1})
    s.request("evaluate", {"expression": "v.name", "frameId": 1, "context": "watch"})
    s.request("continue", {"threadId": 1})
    s.wait("terminated")
    s.request("disconnect", {})
    s.flush_output()
    s.p.stdin.close()
    print("exit", s.p.wait())


if __name__ == "__main__":
    main()
