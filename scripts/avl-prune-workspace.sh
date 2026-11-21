#!/usr/bin/env bash
# Форк для AVL agent: убирает из codex-rs крейты, которые AVL не компилирует.
# AVL использует семь крейтов (см. scripts/avl-check.sh); всё, что не входит в их замыкание
# зависимостей (с dev-зависимостями оставшихся крейтов), удаляется: каталоги, записи в members
# и в [workspace.dependencies]. Скрипт идемпотентен — после перехода на новый тег upstream
# его запускают заново, а не переносят коммит с удалением.
#
#   scripts/avl-prune-workspace.sh           — удалить
#   scripts/avl-prune-workspace.sh --list    — только показать, что вне замыкания (по cargo metadata)
set -euo pipefail
cd "$(dirname "$0")/../codex-rs"

ROOTS="codex-app-server-client codex-app-server-protocol codex-arg0 codex-config codex-core codex-feedback codex-protocol"

outside="$(cargo metadata --format-version 1 | ROOTS="$ROOTS" python3 -c '
import json, os, sys
m = json.load(sys.stdin)
pk = {p["id"]: p for p in m["packages"]}
ws = set(m["workspace_members"])
nodes = {n["id"]: n for n in m["resolve"]["nodes"]}
want = set(os.environ["ROOTS"].split())
stack = [i for i in ws if pk[i]["name"] in want]
missing = want - {pk[i]["name"] for i in stack}
if missing:
    sys.exit("нет корневых крейтов: " + " ".join(sorted(missing)))
seen = set()
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]["deps"]:
        kinds = {k["kind"] for k in d["dep_kinds"]}
        # dev-зависимости учитываем только у крейтов рабочего пространства: без них cargo не разрешит граф
        if kinds & {None, "build"} or ("dev" in kinds and i in ws):
            stack.append(d["pkg"])
root = m["workspace_root"]
for i in sorted(ws - seen, key=lambda i: pk[i]["name"]):
    print(os.path.relpath(os.path.dirname(pk[i]["manifest_path"]), root), pk[i]["name"])
')"

if [ "${1:-}" = "--list" ]; then
  printf '%s\n' "$outside"
  exit 0
fi
[ -n "$outside" ] || { echo "вне замыкания ничего нет"; exit 0; }

while read -r dir name; do
  rm -rf -- "$dir"
  DIR="$dir" NAME="$name" python3 - <<'PY'
import os, re
d, n = os.environ["DIR"], os.environ["NAME"]
src = open("Cargo.toml").read()
out = re.sub(r'^\s*"%s",\n' % re.escape(d), "", src, count=1, flags=re.M)            # members
out = re.sub(r'^%s = \{ path = "%s"[^\n]*\n' % (re.escape(n), re.escape(d)), "", out, flags=re.M)  # workspace.dependencies
open("Cargo.toml", "w").write(out)
PY
  echo "убран $name ($dir)"
done <<<"$outside"
