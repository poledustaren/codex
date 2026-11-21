#!/usr/bin/env bash
# Форк для AVL agent: проверка, что крейты, которые использует AVL, компилируются,
# а вырезанные зависимости не вернулись в дерево.
#
#   scripts/avl-check.sh                      — cargo check семи крейтов + список запретов из scripts/avl-banned.txt
#   scripts/avl-check.sh 'sentry.*' v8        — то же и дополнительно эти имена (регулярные выражения на имя крейта)
#   AVL_CHECK_TREE_ONLY=1 scripts/avl-check.sh — только дерево зависимостей, без компиляции
set -euo pipefail
cd "$(dirname "$0")/../codex-rs"

PKGS=(-p codex-app-server-client -p codex-app-server-protocol -p codex-arg0 -p codex-config -p codex-core -p codex-feedback -p codex-protocol)

banned=("$@")
if [ -f ../scripts/avl-banned.txt ]; then
  while read -r line; do
    line="${line%%#*}"; line="${line//[[:space:]]/}"
    [ -n "$line" ] && banned+=("$line")
  done < ../scripts/avl-banned.txt
fi

# все платформы сразу (--target all): Windows, macOS и Android проверяются так же, как Linux
tree="$(cargo tree "${PKGS[@]}" --target all -e normal,build --prefix none --format '{p}' | awk '{print $1}' | sort -u)"
echo "крейтов в дереве зависимостей AVL: $(printf '%s\n' "$tree" | wc -l)"

fail=0
for pat in ${banned[@]+"${banned[@]}"}; do
  if hit="$(printf '%s\n' "$tree" | grep -E -x -- "$pat")"; then
    echo "ЗАПРЕЩЁННЫЙ крейт в дереве ($pat): $(echo $hit)" >&2
    fail=1
  fi
done
[ "$fail" -eq 0 ] || exit 1
[ ${#banned[@]} -eq 0 ] || echo "запрещённых крейтов нет (${#banned[@]} шаблонов)"

[ -n "${AVL_CHECK_TREE_ONLY:-}" ] && exit 0
cargo check -q "${PKGS[@]}"
echo "AVL CHECK OK"
