#!/usr/bin/env python3
"""Full real dev task on 白山智算 model. plan -> build -> edit -> compile/run -> /init."""
import os, pty, select, subprocess, time, re, sys, struct, fcntl, termios

BIN = os.path.expanduser("~/.cargo/bin/kivio-code")
SBX = open("/tmp/kivio_bs_sbx.txt").read().strip()
MODEL = "provider-1781172614923:DeepSeek-V4-Flash"

m, s = pty.openpty()
fcntl.ioctl(s, termios.TIOCSWINSZ, struct.pack("HHHH", 55, 120, 0, 0))
p = subprocess.Popen([BIN, "--model", MODEL], stdin=s, stdout=s, stderr=s, cwd=SBX, close_fds=True)
os.close(s)
cap = bytearray()
def drain(t):
    e=time.time()+t
    while time.time()<e:
        r,_,_=select.select([m],[],[],0.3)
        if r:
            try: d=os.read(m,65536)
            except OSError: break
            if not d: break
            cap.extend(d)
def t(x): os.write(m,x.encode()); time.sleep(0.5)
def cr(): os.write(m,b"\r"); time.sleep(0.4)
def k(seq): os.write(m,seq); time.sleep(0.5)
def plain(): return re.sub(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[\]_].*?(\x07|\x1b\\)","",bytes(cap).decode("utf-8","replace"))

drain(1.8); print("[*] launched on 白山")
k(b"\x1b[Z"); drain(0.5)            # -> plan
t("Plan (do not edit yet) how to add fn greet(name:&str) printing \"Hello, <name>!\" and call greet(\"Kivio\") in main. Keep it to 3 short steps."); cr()
print("[*] T1 plan…"); drain(75)
plan_txt = plain()
k(b"\x1b[Z"); drain(0.5)            # -> build
t("proceed: make that edit to main.rs now using the edit tool."); cr()
print("[*] T2 edit…"); drain(85)
t("Now compile main.rs with rustc to /tmp/greet_bin and run it; show the exact output."); cr()
print("[*] T3 compile+run…"); drain(95)
t("/init"); cr(); print("[*] /init…"); drain(85)
t("/quit"); cr(); drain(2)
for _ in range(30):
    if p.poll() is not None: break
    time.sleep(0.1)
if p.poll() is None: p.kill()
os.close(m)
pl=plain(); low=pl.lower()
# distinct ordered lines for readable transcript
seen=[]
for l in pl.splitlines():
    x=l.rstrip()
    if x.strip() and (not seen or seen[-1]!=x): seen.append(x)
print("\n=== TRANSCRIPT (distinct lines, tool cards + answers) ===")
for x in seen:
    s2=x.strip()
    if any(t2 in s2 for t2 in ["✓","✗","plan","Plan","greet","rustc","Hello","read","edit","write","bash","→","Step","step","计划","错误","余额","402","💭","thought","KIVIO"]) or s2.startswith(("-","1.","2.","3.","$")):
        print("  ", s2[:116])
print("\n=== CHECKS ===")
err = [w for w in ["余额","402","请求失败","鉴权"] if w in pl]
print("  provider error:", err or "none")
print("  edit/write tool card (✓ edit|write):", bool(re.search(r"[✓✗]\s*(edit|write)", pl)))
print("  bash tool card (✓ bash):", bool(re.search(r"[✓✗]\s*bash", pl)))
print("\n=== FILES ===")
print("--- main.rs ---"); print(open(os.path.join(SBX,"main.rs")).read())
kp=os.path.join(SBX,"KIVIO.md")
print("KIVIO.md exists:", os.path.exists(kp), "size:", os.path.getsize(kp) if os.path.exists(kp) else 0)
print("exit:", p.returncode)
