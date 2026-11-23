#!/bin/bash
# S6 local matrix under srt in WSL2. Synthetic markers only; no provider calls.
export PATH="$HOME/node/bin:$HOME/.npm-global/bin:/usr/local/bin:/usr/bin:/bin"
B=$HOME/s6; rm -rf "$B"; mkdir -p "$B/wt" "$B/sibling" "$B/remote.git"
echo SYNTHETIC-MARKER > "$B/sibling/marker.txt"
git init -q --bare "$B/remote.git"
cd "$B/wt" && git init -q && git config user.email a@b.c && git config user.name t \
  && git remote add origin "$B/remote.git" && echo hi > f.txt && git add . && git commit -qm init
cp "$B/wt/.git/config" "$B/config.orig"
# host loopback listener (control)
python3 -m http.server 8099 --bind 127.0.0.1 --directory "$B/sibling" >/dev/null 2>&1 &
LP=$!; sleep 1
curl -s -o /dev/null -w "host-control loopback: %{http_code}\n" http://127.0.0.1:8099/marker.txt

cat > "$B/settings.json" <<EOF
{"network":{"allowedDomains":["api.anthropic.com"],"deniedDomains":[]},
 "filesystem":{"denyRead":["$HOME"],"allowRead":["$B/wt","$B/probe.sh","$HOME/.npm-global/lib/node_modules/@anthropic-ai/sandbox-runtime/vendor/seccomp"],
  "allowWrite":["$B/wt"],"denyWrite":["$B/wt/.git/config","$B/wt/.git/hooks"]}}
EOF

cat > "$B/probe.sh" <<'EOF'
#!/bin/bash
B=$HOME/s6
r() { if "$@" >/dev/null 2>&1; then echo "ALLOWED  $LABEL"; else echo "DENIED   $LABEL"; fi; }
LABEL="write inside worktree"            r bash -c "echo x > $B/wt/out.txt"
LABEL="write outside worktree (sibling)" r bash -c "echo x > $B/sibling/new.txt"
LABEL="write outside worktree (/tmp/x)"  r bash -c "echo x > /tmp/x-s6-probe"
LABEL="read sibling marker"              r cat $B/sibling/marker.txt
LABEL="read ~/.ssh or home dotfile"      r cat $HOME/.profile
LABEL="append .git/config"               r bash -c "echo '[x]' >> $B/wt/.git/config"
LABEL="write .git/hooks/pre-push"        r bash -c "echo x > $B/wt/.git/hooks/pre-push"
LABEL="git push to local bare remote"    r git -C $B/wt push origin HEAD:refs/heads/x
LABEL="loopback host listener 127.0.0.1:8099" r curl -s -m 5 http://127.0.0.1:8099/marker.txt
LABEL="allowed domain api.anthropic.com" r curl -s -m 10 -o /dev/null https://api.anthropic.com/
LABEL="denied domain example.com"        r curl -s -m 10 -o /dev/null https://example.com/
LABEL="raw tcp to 1.1.1.1:443 (no proxy)" r bash -c "unset HTTPS_PROXY HTTP_PROXY ALL_PROXY https_proxy http_proxy all_proxy; curl -s -m 5 --noproxy '*' -o /dev/null https://1.1.1.1/"
LABEL="gh binary"                        r which gh
if [ -n "$GH_TOKEN" ]; then echo "VISIBLE  GH_TOKEN in env"; else echo "ABSENT   GH_TOKEN in env"; fi
if [ -n "$ANTHROPIC_API_KEY" ]; then echo "VISIBLE  ANTHROPIC_API_KEY in env"; else echo "ABSENT   ANTHROPIC_API_KEY in env"; fi
EOF
chmod +x "$B/probe.sh"

echo "=== A: probe directly, env NOT scrubbed (srt inherits) ==="
GH_TOKEN=synthetic-gh ANTHROPIC_API_KEY=synthetic-key srt -s "$B/settings.json" -c "bash $B/probe.sh" 2>&1
echo "=== B: descendant (probe launched from a child of a child), env scrubbed by launcher ==="
env -i PATH="$PATH" HOME="$HOME" srt -s "$B/settings.json" -c "bash -c 'sh -c \"bash $B/probe.sh\"'" 2>&1
echo "=== host verification after run ==="
cmp -s "$B/config.orig" "$B/wt/.git/config" && echo "git config unchanged" || echo "GIT CONFIG CHANGED"
[ "$(cat $B/sibling/marker.txt)" = SYNTHETIC-MARKER ] && echo "marker unchanged" || echo "MARKER CHANGED"
[ -e "$B/sibling/new.txt" ] && echo "SIBLING WRITE LANDED" || echo "no sibling write"
[ -e /tmp/x-s6-probe ] && echo "TMP WRITE LANDED" || echo "no /tmp write"
[ -e "$B/wt/.git/hooks/pre-push" ] && echo "HOOK WRITE LANDED" || echo "no hook write"
git -C "$B/remote.git" branch --list x | grep -q x && echo "PUSH LANDED" || echo "no push landed"
[ -e "$B/wt/out.txt" ] && echo "worktree write landed (expected)"
kill $LP
