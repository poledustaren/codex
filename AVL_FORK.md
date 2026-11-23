# Форк Codex для AVL agent

Ветка `avl/<версия>` — это тег upstream `rust-v<версия>` (openai/codex) минус то, что AVL agent
не использует. AVL встраивает Codex в свой процесс (`InProcessAppServerClient`) и берёт из
`codex-rs` семь крейтов:

`codex-app-server-client`, `codex-app-server-protocol`, `codex-arg0`, `codex-config`, `codex-core`,
`codex-feedback`, `codex-protocol`.

Всё остальное в репозитории нужно ровно настолько, насколько от него зависят эти семь.

## Что убрано

| Что | Как | Зачем |
|---|---|---|
| 38 крейтов вне замыкания зависимостей (tui, cli, exec, cloud-tasks*, code-mode-host/runtime, ollama, lmstudio, realtime-webrtc, v8-poc, voice-host и др.) | каталоги удалены скриптом `scripts/avl-prune-workspace.sh` | AVL их не компилирует; меньше поверхность при слиянии |
| OpenTelemetry (`opentelemetry*`, `tracing-opentelemetry`) | `codex-otel` — заглушка с тем же публичным API: метрики и трассировка ничего не пишут и никуда не уходят | телеметрия AVL не нужна, наружу ничего отправляться не должно |
| Sentry в `codex-feedback` | отправка отчёта возвращает ошибку, сбор логов для отчёта остался | отчёты в чужой Sentry не нужны |
| Amazon Bedrock (`codex-aws-auth`, `aws-config`, `aws-sdk-*`) | провайдер удалён; выбор Bedrock и RPC настройки Bedrock отвечают ошибкой «не поддерживается в этой сборке» | AVL ходит в свой шлюз моделей |
| Декодер аудио `symphonia` | токены аудиовложения оцениваются по размеру data URL | голос в AVL идёт мимо Codex |
| Миграция из чужих агентов (`codex-external-agent-migration`) | RPC `externalAgentConfig/*` отвечают ошибкой | в AVL нет импорта настроек чужих агентов |
| Аналитика (`codex-analytics`) | очередь доставки не создаётся никогда, все `track_*` пустые, какой бы ни была настройка `analytics.enabled` | события не должны уходить в OpenAI |
| `mxc-sdk` вне Windows | зависимость перенесена под `cfg(windows)` | под Android и Linux он не собирался, AVL патчил его скриптом |

## Что не тронуто намеренно

Shell и всё, чем агент работает: исполнение команд (`exec-server`, `unified exec`, PTY), песочницы
(landlock/bubblewrap, seatbelt, Windows), политика исполнения (`execpolicy`, starlark), сетевой
прокси песочницы (`network-proxy`, в том числе его переподпись запросов `aws-sigv4` — это часть
прокси, а не Bedrock), применение патчей, MCP-клиент и сервер, инструменты, навыки, память,
guardian, code-mode, вход (`codex-login`), хранилище секретов (keyring), git-утилиты, состояние
и журнал сессий.

## Принцип: заглушка на шве

Вырезаемая подсистема заменяется пустой реализацией там, где она соединяется с остальным кодом:
имя крейта и его публичный API остаются, вызывающий код почти не меняется. Так дифф относительно
upstream остаётся маленьким, а следующий тег вливается без переписывания сотен мест вызова.
Удалять вызовы по всему коду нужно только там, где иначе не уходит тяжёлая зависимость.

## Проверка

```bash
scripts/avl-check.sh                        # дерево зависимостей семи крейтов + cargo check
AVL_CHECK_TREE_ONLY=1 scripts/avl-check.sh  # только дерево, без компиляции
```

Скрипт считает дерево сразу для всех платформ (`--target all`) и падает, если в нём появился крейт
из `scripts/avl-banned.txt`. Вернулась вырезанная зависимость после слияния — проверка это покажет.

Экономия диска: полная проверка с тестами (`cargo check --workspace --all-targets`) без ограничений
занимает в `target` десятки гигабайт. Запускать так:

```bash
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 cargo check ...
```

## Переход на новый тег upstream

```bash
git fetch upstream --tags
git switch -c avl/<новая версия> avl/<старая версия>
git merge rust-v<новая версия>          # конфликты — в основном в вырезанных местах
scripts/avl-prune-workspace.sh --list   # что снова оказалось вне замыкания
scripts/avl-prune-workspace.sh          # удалить; скрипт идемпотентен
scripts/avl-check.sh
```

Конфликт в удалённом у нас файле решается в пользу удаления. Новый код upstream, который зовёт
вырезанное (`codex_otel`, Bedrock, Sentry), чинится на шве: недостающий метод добавляется в заглушку.

В AVL agent версия форка задаётся в `crates/avl-codex/Cargo.toml` (семь зависимостей с
`git = ".../poledustaren/codex", branch = "avl/<версия>"`), затем `cargo update -p codex-core`.

## Замеры (0.169.1)

| | upstream | форк |
|---|---|---|
| Крейтов в дереве зависимостей семи крейтов | 1023 | 979 |
| Крейтов в дереве `avl-codex` | 1062 | 1018 |

Тесты вырезанных подсистем удалены; `cargo check --workspace --all-targets` проходит.

## Вес сборки

Бинарь `avl-agent` (Linux x64, `cargo build --release -p avl-agent`, 12 ядер), размер / размер после `strip`:

| Профиль release | upstream 0.169.1 | форк | Время сборки |
|---|---|---|---|
| штатный AVL (`codegen-units = 4`) | 355,4 / 254,5 МБ | 333,6 / 241,3 МБ | 14 мин |
| `opt-level = "s"`, `lto = "thin"` | 260,6 / 162,7 МБ | — | 11 мин |
| `opt-level = "s"`, `lto = "fat"`, `codegen-units = 1` | 140,4 / 108,0 МБ | — | 27 мин |
| `opt-level = "z"`, `lto = "fat"`, `codegen-units = 1` | 136,0 / 99,3 МБ | 130,1 / 95,1 МБ | 16–24 мин |

e2e AVL agent с Codex (`scripts/e2e.sh codex`) проходит на форке и со штатным профилем, и с `z` + fat LTO.

Вывод: вырезанное даёт около 13 МБ (5%). Почти весь вес — ядро агента и сгенерированная
сериализация протокола (serde), и снижает его профиль сборки, а не удаление подсистем.
Побочный эффект `opt-level = "z"` — более медленный числовой код на CPU; скорость не замерялась.

## Расширения форка для AVL agent

- `thread/resume.dynamicTools` (`ThreadResumeParams.dynamic_tools` + `StartThreadOptions.override_dynamic_tools`): позволяет обновлять каталог динамических инструментов (включая очистку до пустого списка `[]`) при `thread/resume`, когда пользователь включает/выключает MCP-серверы или скиллы в существующем чате.

## Что ещё можно вырезать (не сделано)

Кандидаты, которые либо относятся к механике агента, либо глубоко вплетены — резать только
по отдельному решению:

- `network-proxy` на `rama` (~7 МБ кода) — сетевой прокси песочницы;
- `keyring`/`zbus` (~4 МБ) и `secrets`/`age` (~3 МБ) — хранение секретов;
- `execpolicy` на starlark, `git-utils` на gix, `code-mode` (tonic/protoc);
- `cloud-client`, `cloud-config`, `connectors`, `message-board`, `memories`, `guardian`;
- под Windows — libcurl с NTLM (локально не проверить).
