# Как запустить — Log Analyzer

## Требования

- Rust (stable)
- Bash (для `.sh`-скриптов генерации логов)

## Сборка

```bash
cargo build --release
```

---

## Базовое использование

```bash
# Один файл
cargo run -- logs/test.log

# Несколько файлов
cargo run -- logs/test.log logs/test_scenarios/error_spike.log

# С конфигом
cargo run -- -c test_config.json logs/test.log

# С фильтром (применяется до статистики — отфильтрованные строки не попадают в счётчики)
cargo run -- --filter "level=ERROR" logs/test.log
cargo run -- --filter "level=ERROR,keyword=failed,source=auth" logs/test.log

# С N воркерами
cargo run -- -w 8 logs/test.log
```

---

## Синтаксис фильтра

`--filter "key=value,key=value,..."`

| Ключ | Пример | Описание |
|---|---|---|
| `level` | `level=ERROR` | Фильтр по уровню лога |
| `keyword` | `keyword=timeout` | Поиск подстроки в сообщении |
| `source` | `source=auth` | Фильтр по параметру `service=` |

---

## Сессии

```bash
# Создать сессию и запустить анализ
cargo run -- -s my_session logs/test.log

# Возобновить сохранённую сессию
cargo run -- -s my_session

# Список сессий
cargo run -- list-sessions

# Информация о сессии
cargo run -- stats -s my_session

# Удалить сессию
cargo run -- remove-session my_session
```

---

## Конфигурационные файлы

Поддерживаемые форматы: **JSON**, **YAML**, **TOML**.

```bash
# Проверить конфиг без запуска анализа
cargo run -- load-config -c test_config.json
cargo run -- load-config -c test_config.yaml
cargo run -- load-config -c test_config.toml
```

### Пример конфига

```json
{
  "rules": [
    {
      "name": "connection_issues",
      "pattern": "connection|timeout",
      "action": "report",
      "severity": "error",
      "enabled": true,
      "threshold": 5,
      "time_window": 60
    }
  ],
  "settings": {
    "workers": 4,
    "anomaly_threshold": 15,
    "output_format": "text"
  }
}
```

Действия: `count` · `warn` · `report` · `ignore`

### Полное описание полей конфига

#### Блок `rules` — список правил анализа

Каждое правило — это объект со следующими полями:

| Поле | Тип | Обязательное | По умолчанию | Описание |
|---|---|---|---|---|
| `name` | string | да | — | Уникальное имя правила. Отображается в итоговом отчёте. |
| `pattern` | string | да | — | Regex-паттерн для поиска в тексте сообщения. Например: `"ERROR\|CRITICAL"` или `"timeout"`. |
| `action` | string | нет | `"count"` | Что делать при срабатывании (см. ниже). |
| `severity` | string | нет | `"info"` | Метка серьёзности: `"info"`, `"warning"`, `"error"`, `"critical"`. Используется в отчёте. |
| `enabled` | bool | нет | `true` | Включить или выключить правило без удаления из файла. |
| `threshold` | число | нет | не задан | Минимальное число совпадений за `time_window` секунд, после которого правило срабатывает. Если не задан — правило срабатывает на каждом совпадении. |
| `time_window` | число (сек) | нет | не задан | Ширина скользящего окна в секундах для счётчика `threshold`. Используется вместе с `threshold`. |

**Значения `action`:**

| Значение | Поведение |
|---|---|
| `count` | Молча считает совпадения. Число видно в итоговой статистике. |
| `warn` | Выводит `WARN` в лог (`RUST_LOG=warn` чтобы увидеть в реальном времени). |
| `report` | Добавляет совпадение в итоговый отчёт, который печатается по завершении анализа. |
| `ignore` | Полностью игнорирует строку при совпадении — она не попадает ни в какую статистику. |

**Пример с порогом:** правило ниже сработает только если слово `timeout` встретилось 5 и более раз за 60 секунд:

```json
{
  "name": "timeout_burst",
  "pattern": "timeout",
  "action": "report",
  "severity": "error",
  "threshold": 5,
  "time_window": 60
}
```

---

#### Блок `settings` — глобальные настройки

| Поле | Тип | По умолчанию | Описание |
|---|---|---|---|
| `workers` | число | `4` | Количество параллельных потоков-обработчиков. Увеличивайте при большом числе правил или тяжёлых regex. |
| `buffer_size` | число (байт) | `1024` | Размер буфера чтения файла. Увеличьте до `4096`–`16384` для больших файлов. |
| `output_format` | string | `"text"` | Формат итогового вывода: `"text"` — читаемый текст, `"json"` — компактный JSON, `"pretty"` — отформатированный JSON. |
| `anomaly_threshold` | число | `20` | Порог для встроенного детектора аномалий: если одно и то же сообщение повторяется ≥ N раз в скользящем окне 60 с — фиксируется аномалия `RepeatedMessage`; если ошибок ≥ N — `ErrorSpike`; если всего сообщений ≥ N·10 — `MessageFlood`. |
| `report_dir` | string | не задан | Папка для сохранения отчётов в файл. Если не задан — отчёт только в stdout. |

**Минимальный рабочий конфиг (JSON):**

```json
{
  "rules": [
    {
      "name": "errors",
      "pattern": "ERROR|CRITICAL",
      "action": "report"
    }
  ],
  "settings": {
    "workers": 4
  }
}
```

Все остальные поля необязательны и имеют разумные значения по умолчанию.

---

## Тестирование обнаружения аномалий

Скрипты в папке `logs/` записывают данные в `logs/test.log` и воспроизводят конкретный тип аномалии.

### Генерация и тестирование каждой аномалии

```bash
# RepeatedMessage — одно и то же сообщение 25 раз
bash logs/gen_repeated_message.sh
cargo run -- -c logs/test_scenarios/comprehensive_config.json logs/test.log

# ErrorSpike — 25 разных строк с уровнем ERROR/CRITICAL
bash logs/gen_error_spike.sh
cargo run -- -c logs/test_scenarios/comprehensive_config.json logs/test.log

# MessageFlood — 220 строк суммарно
bash logs/gen_message_flood.sh
cargo run -- -c logs/test_scenarios/comprehensive_config.json logs/test.log

# Сгенерировать все три сразу (последний скрипт перезаписывает logs/test.log)
bash logs/gen_all.sh
```

Пороги аномалий в `comprehensive_config.json`: `anomaly_threshold = 15`, окно = 60 с.

| Аномалия | Условие срабатывания |
|---|---|
| `RepeatedMessage` | Одно сообщение ≥ 15 раз за 60 с |
| `ErrorSpike` | ERROR/CRITICAL ≥ 15 за 60 с |
| `MessageFlood` | Всего сообщений ≥ 150 за 60 с |

---

## Запуск тестов

```bash
cargo test
cargo test -- --nocapture                    # с выводом stdout
RUST_LOG=debug cargo test -- --nocapture     # с отладочными логами
cargo test statistics                        # один модуль
```

---

## Формат лога

```
YYYY-MM-DD HH:MM:SS LEVEL Сообщение [key=value ...]
```

```
2024-01-15 10:30:00 INFO  App started [service=auth]
2024-01-15 10:30:01 ERROR DB connection failed [service=db host=localhost]
2024-01-15 10:30:02 WARN  High memory [service=monitor memory=85%]
```

Поддерживаемые уровни: `DEBUG` · `INFO` · `WARN` / `WARNING` · `ERROR` · `CRITICAL`
