#!/usr/bin/env python3
"""Drive kivio-code through a REAL multi-turn dev task in a sandbox, like a human.
Scenario:
  T1 (plan mode): ask for a plan to add a `greet(name)` fn + use it. (read-only)
  switch to build (Shift+Tab)
  T2 (build): "proceed" -> it should edit main.rs to add greet + call it
  T3 (build): "run it with rustc and show output" -> bash compile+run
  T4: /init -> writes KIVIO.md
Captures the transcript and checks tool usage + that files actually changed.
"""
import os, pty, select, subprocess, time, re, sys, struct, fcntl, termios

BIN = os.path.expanduser("~/.cargo/bin/kivio-code")
SBX = sys.argv[1]

m, s = pty.openpty()
fcntl.ioctl(s, termios.TIOCSWINSZ, struct.pack("HHHH", 50, 120, 0, 0))
p = subprocess.Popen([BIN], stdin=s, stdout=s, stderr=s, cwd=SBX, close_fds=True)
os.close(s)
cap = bytearray()

def drain(t):
    end = time.time() + t
    while time.time() < end:
        r,_,_ = select.select([m], [], [], 0.3)
        if r:
            try: d = os.read(m, 65536)
            except OSError: break
            if not d: break
            cap.extend(d)

def send_text(x):
    os.write(m, x.encode()); time.sleep(0.5)
def enter():
    os.write(m, b"\r"); time.sleep(0.4)
def key(seq):
    os.write(m, seq); time.sleep(0.5)

def plain():
    return re.sub(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[\]_].*?(\x07|\x1b\\)", "", bytes(cap).decode("utf-8","replace"))

drain(1.8)
print("[*] launched")
# T1: plan mode
key(b"\x1b[Z")  # Shift+Tab -> plan
drain(0.6)
send_text("Plan how to add a greet(name: &str) function to main.rs and call it from main with name \"Kivio\". Keep it short.")
enter(); print("[*] T1 plan sent, waiting…"); drain(70)
# switch to build
key(b"\x1b[Z")  # Shift+Tab -> build
drain(0.6); print("[*] switched to build")
# T2: proceed (edit file)
send_text("proceed: implement that change in main.rs now."); enter()
print("[*] T2 proceed sent, waiting…"); drain(80)
# T3: compile + run
send_text("Compile main.rs with rustc and run it; show me the output."); enter()
print("[*] T3 compile+run sent, waiting…"); drain(90)
# T4: /init
send_text("/init"); enter(); print("[*] /init sent, waiting…"); drain(80)
send_text("/quit"); enter(); drain(2)
for _ in range(30):
    if p.poll() is not None: break
    time.sleep(0.1)
if p.poll() is None: p.kill()
os.close(m)

pl = plain()
low = pl.lower()
# heuristic checks
checks = {
  "plan-mode chip seen": " plan " in pl,
  "build-mode chip seen": " build " in pl,
  "edit/write tool used": ("edit" in low or "write" in low) and ("main.rs" in pl),
  "bash/rustc used": ("bash" in low or "rustc" in low or "running:" in low),
  "greet in transcript": "greet" in low,
}
print("\n=== TAIL (last 1800 chars) ===")
print(pl[-1800:])
print("\n=== CHECKS ===")
for k,v in checks.items(): print(f"  {'OK ' if v else 'NO '} {k}")
print("\n=== FILES ON DISK ===")
print("--- main.rs ---");
try: print(open(os.path.join(SBX,"main.rs")).read())
except Exception as e: print("(err)", e)
print("KIVIO.md exists:", os.path.exists(os.path.join(SBX,"KIVIO.md")))
print("exit code:", p.returncode)
