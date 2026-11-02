# Runs inside one SSH session; no installation or repository writes.
set -u
umask 077
work=$(mktemp -d) || exit 1
trap 'rm -rf "$work"' EXIT HUP INT TERM
while IFS= read -r length; do
    case "$length" in ''|*[!0-9]*) exit 2 ;; esac
    head -c "$length" > "$work/request" || exit 2
    sh "$work/request" </dev/null >"$work/out" 2>"$work/err"
    status=$?
    printf '%s %s %s\n' "$status" "$(wc -c < "$work/out")" "$(wc -c < "$work/err")"
    cat "$work/out" "$work/err" || exit 2
done
