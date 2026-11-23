#!/bin/bash
# S6 gate: worktree on drvfs (/mnt/d), Windows interop escapes. Synthetic only, no provider calls.
export PATH="$HOME/node/bin:$HOME/.npm-global/bin:/usr/local/bin:/usr/bin:/bin"
B=/mnt/d/autopilot-s6-gate; rm -rf "$B"; mkdir -p "$B/wt" "$B/sibling" "$B/remote.git"
echo SYNTHETIC-MARKER > "$B/sibling/marker.txt"
git init -q --bare "$B/remote.git"
cd "$B/wt" && git init -q && git config user.email a@b.c && git config user.name t \
  && git remote add origin "$B/remote.git" && echo hi > f.txt && git add . && git commit -qm init
# host-side (outside sandbox): a Windows exe placed inside the worktree
cp /mnt/c/Windows/System32/cmd.exe "$B/wt/cmd.exe"
cp "$B/wt/.git/config" "$B/config.orig"
python3 -m http.server 8099 --bind 127.0.0.1 --directory "$B/sibling" >/dev/null 2>&1 &
LP=$!; sleep 1
echo "interop on host side: $(cat /proc/sys/fs/binfmt_misc/WSLInterop* 2>/dev/null | head -1) WSL_INTEROP=${WSL_INTEROP:-unset}"
cat > "$B/settings.json" <<EOF
{"network":{"allowedDomains":["api.anthropic.com"],"deniedDomains":[]},
 "filesystem":{"denyRead":["$HOME","/mnt"],
  "allowRead":["$B/wt","$B/probe.sh","$HOME/.npm-global/lib/node_modules/@anthropic-ai/sandbox-runtime/vendor/seccomp"],
  "allowWrite":["$B/wt"],
  "denyWrite":["$B/wt/.git/config","$B/wt/.git/hooks"]}}
EOF
cat > "$B/probe.sh" <<'EOF'
#!/bin/bash
B=/mnt/d/autopilot-s6-gate
r() { if "$@" >/dev/null 2>&1; then echo "ALLOWED  $LABEL"; else echo "DENIED   $LABEL"; fi; }
LABEL="write inside worktree (drvfs)"     r bash -c "echo x > $B/wt/out.txt"
LABEL="write sibling folder (drvfs)"      r bash -c "echo x > $B/sibling/new.txt"
LABEL="read sibling marker (drvfs)"       r cat $B/sibling/marker.txt
LABEL="read other Windows drive file"     r cat /mnt/c/Windows/win.ini
LABEL="append .git/config"                r bash -c "echo '[x]' >> $B/wt/.git/config"
LABEL="write .git/hooks/pre-push"         r bash -c "echo x > $B/wt/.git/hooks/pre-push"
LABEL="git push to local bare remote"     r git -C $B/wt push origin HEAD:refs/heads/x
LABEL="loopback host listener"            r curl -s -m 5 http://127.0.0.1:8099/marker.txt
LABEL="api.anthropic.com"                 r curl -s -m 10 -o /dev/null https://api.anthropic.com/
LABEL="example.com"                       r curl -s -m 10 -o /dev/null https://example.com/
echo "--- interop ---"
LABEL="exec /mnt/c/Windows/System32/cmd.exe by absolute path" r /mnt/c/Windows/System32/cmd.exe /c "echo x > D:\\autopilot-s6-gate\\sibling\\pwned1.txt"
LABEL="exec cmd.exe copied into the worktree"                 r $B/wt/cmd.exe /c "echo x > D:\\autopilot-s6-gate\\sibling\\pwned2.txt"
LABEL="exec /init directly"                                   r /init
LABEL="cmd.exe via PATH"                                      r bash -c "cmd.exe /c echo x"
echo "WSL_INTEROP=${WSL_INTEROP:-unset}"; ls /run/WSL 2>&1 | head -3
EOF
srt -s "$B/settings.json" -c "bash $B/probe.sh" 2>&1
echo "=== host verification ==="
cmp -s "$B/config.orig" "$B/wt/.git/config" && echo "git config unchanged" || echo "GIT CONFIG CHANGED"
[ "$(cat $B/sibling/marker.txt)" = SYNTHETIC-MARKER ] && echo "marker unchanged" || echo "MARKER CHANGED"
for f in new.txt pwned1.txt pwned2.txt; do [ -e "$B/sibling/$f" ] && echo "ESCAPE LANDED: $f" || echo "no $f"; done
[ -e "$B/wt/.git/hooks/pre-push" ] && echo "HOOK WRITE LANDED" || echo "no hook write"
git -C "$B/remote.git" branch --list x | grep -q x && echo "PUSH LANDED" || echo "no push landed"
kill $LP
