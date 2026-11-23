#!/bin/bash
export PATH="$HOME/node/bin:$HOME/.npm-global/bin:/usr/local/bin:/usr/bin:/bin"
B=/mnt/d/autopilot-s6-gate
echo "--- control: same copied cmd.exe, OUTSIDE the sandbox ---"
$B/wt/cmd.exe /c "echo x > D:\\autopilot-s6-gate\\sibling\\control.txt" < /dev/null 2>&1 | head -2
[ -e $B/sibling/control.txt ] && echo "control landed: the interop test can detect an escape" || echo "control did NOT land: interop test is blind"
rm -f $B/sibling/control.txt
echo "--- unix socket to the interop socket ---"
python3 - <<'EOF'
import socket,glob
for p in sorted(glob.glob("/run/WSL/*_interop")):
    s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
    try: s.connect(p); print("host connect ok", p)
    except Exception as e: print("host connect failed", p, e)
    break
EOF
cat > $B/s.json <<EOF
{"network":{"allowedDomains":[],"deniedDomains":[]},"filesystem":{"denyRead":["/mnt"],"allowRead":["$B/wt","$B/sock.py"],"allowWrite":[],"denyWrite":[]}}
EOF
cat > $B/sock.py <<'EOF'
import socket,glob
for p in sorted(glob.glob("/run/WSL/*_interop")):
    s=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
    try: s.connect(p); print("SANDBOX connect ok", p)
    except Exception as e: print("sandbox connect failed", p, e)
    break
EOF
srt -s $B/s.json -c "python3 $B/sock.py" 2>&1 | tail -3
