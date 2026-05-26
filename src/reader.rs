//! Модуль потокового чтения логов (Reader)
//! Читает файлы по мере появления новых строк, не загружая весь файл в память

use crate::error::{LogAnalyzerError, Result};
use crate::log_parser::{quick_parse, LogEntry};
use crate::statistics::SharedStatistics;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Сообщение для передачи между reader и analyzer
#[derive(Debug, Clone)]
pub struct LogMessage {
    /// Распарсенная запись лога
    pub entry: LogEntry,
    /// Время чтения
    pub read_at: chrono::DateTime<chrono::Local>,
}

/// Состояние файла для отслеживания позиции чтения
#[derive(Debug)]
struct FileState {
    /// Путь к файлу
    path: PathBuf,
    /// Текущая позиция в файле
    position: u64,
    /// Размер файла при последнем чтении
    size: u64,
}

/// Reader для потокового чтения логов
pub struct LogReader {
    /// Канал для отправки распарсенных записей
    sender: Sender<LogMessage>,
    /// Файлы для чтения
    files: Vec<PathBuf>,
    /// Состояния файлов
    file_states: HashMap<PathBuf, FileState>,
    /// Флаг остановки
    running: Arc<AtomicBool>,
    /// Счётчик прочитанных строк
    lines_read: Arc<AtomicU64>,
    /// Фильтр для логов
    filter: Option<LogFilter>,
    /// Статистика — обновляется прямо здесь, до отправки в канал
    statistics: Option<SharedStatistics>,
}

impl LogReader {
    /// Создание нового reader
    pub fn new(sender: Sender<LogMessage>, files: Vec<PathBuf>) -> Self {
        Self::with_filter(sender, files, None)
    }

    /// Создание нового reader с фильтром
    pub fn with_filter(sender: Sender<LogMessage>, files: Vec<PathBuf>, filter: Option<LogFilter>) -> Self {
        let mut file_states = HashMap::new();

        // Инициализируем состояние для каждого файла
        for file_path in &files {
            file_states.insert(
                file_path.clone(),
                FileState {
                    path: file_path.clone(),
                    position: 0,
                    size: 0,
                },
            );
        }

        Self {
            sender,
            files,
            file_states,
            running: Arc::new(AtomicBool::new(false)),
            lines_read: Arc::new(AtomicU64::new(0)),
            filter,
            statistics: None,
        }
    }

    /// Подключение статистики — должна вызываться до start()
    pub fn set_statistics(&mut self, stats: SharedStatistics) {
        self.statistics = Some(stats);
    }

    /// Запуск потокового чтения в отдельном потоке
    pub fn start(&mut self) -> JoinHandle<Result<()>> {
        let running = Arc::clone(&self.running);
        let sender = self.sender.clone();
        let files = self.files.clone();
        let lines_read = Arc::clone(&self.lines_read);
        let filter = self.filter.clone();
        let statistics = self.statistics.clone();

        tokio::spawn(async move {
            running.store(true, Ordering::Relaxed);

            // Открываем файлы и читаем существующее содержимое
            let mut file_handles: HashMap<PathBuf, BufReader<File>> = HashMap::new();

            for file_path in &files {
                if let Ok(file) = File::open(file_path) {
                    let reader = BufReader::new(file);
                    file_handles.insert(file_path.clone(), reader);
                }
            }

            // Читаем существующее содержимое
            for (file_path, reader) in &mut file_handles {
                if let Err(e) = read_existing_content(
                    reader, file_path, &sender, &lines_read, filter.as_ref(), statistics.as_ref(),
                )
                .await
                {
                    log::warn!("Error reading existing content from {:?}: {}", file_path, e);
                }
            }

            // Основной цикл чтения новых строк
            while running.load(Ordering::Relaxed) {
                for file_path in &files {
                    if let Some(reader) = file_handles.get_mut(file_path) {
                        match read_new_lines(
                            reader, file_path, &sender, &lines_read, filter.as_ref(), statistics.as_ref(),
                        )
                        .await
                        {
                            Ok(_) => {}
                            Err(e) => {
                                log::debug!("Error reading from {:?}: {}", file_path, e);
                                if let Ok(file) = File::open(file_path) {
                                    *reader = BufReader::new(file);
                                }
                            }
                        }
                    } else if let Ok(file) = File::open(file_path) {
                        let reader = BufReader::new(file);
                        file_handles.insert(file_path.clone(), reader);
                    }
                }

                // Небольшая пауза чтобы не нагружать CPU
                tokio::time::sleep(Duration::from_millis(100)).await;
            }

            Ok(())
        })
    }

    /// Остановка чтения
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Проверка статуса работы
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Получение количества прочитанных строк
    pub fn get_lines_read(&self) -> u64 {
        self.lines_read.load(Ordering::Relaxed)
    }

    /// Добавление нового файла для чтения
    pub fn add_file(&mut self, path: PathBuf) {
        if !self.files.contains(&path) {
            let path_clone = path.clone();
            self.files.push(path);
            self.file_states.insert(
                path_clone.clone(),
                FileState {
                    path: path_clone,
                    position: 0,
                    size: 0,
                },
            );
        }
    }
}

/// Чтение существующего содержимого файла
async fn read_existing_content(
    reader: &mut BufReader<File>,
    file_path: &Path,
    sender: &Sender<LogMessage>,
    lines_read: &Arc<AtomicU64>,
    filter: Option<&LogFilter>,
    statistics: Option<&SharedStatistics>,
) -> Result<()> {
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if let Some(entry) = quick_parse(&line, file_path.to_str().unwrap_or("")) {
                    // Применяем фильтр если он есть
                    if let Some(f) = filter {
                        if !f.matches(&entry) {
                            continue;
                        }
                    }

                    // Обновляем статистику здесь — до отправки в канал,
                    // чтобы счётчики всегда были актуальны (даже если канал полон)
                    if let Some(stats) = statistics {
                        stats.process_entry(&entry);
                    }

                    let msg = LogMessage {
                        entry,
                        read_at: chrono::Local::now(),
                    };

                    if sender.send(msg).is_err() {
                        return Err(LogAnalyzerError::Channel("Receiver disconnected".to_string()));
                    }
                    lines_read.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(e) => return Err(LogAnalyzerError::Io(e)),
        }
    }

    Ok(())
}

/// Чтение новых строк из файла
async fn read_new_lines(
    reader: &mut BufReader<File>,
    file_path: &Path,
    sender: &Sender<LogMessage>,
    lines_read: &Arc<AtomicU64>,
    filter: Option<&LogFilter>,
    statistics: Option<&SharedStatistics>,
) -> Result<()> {
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    if let Some(entry) = quick_parse(trimmed, file_path.to_str().unwrap_or("")) {
                        // Применяем фильтр если он есть
                        if let Some(f) = filter {
                            if !f.matches(&entry) {
                                continue;
                            }
                        }

                        // Обновляем статистику сразу при чтении новой строки —
                        // счётчики обновятся даже если канал окажется переполнен
                        if let Some(stats) = statistics {
                            stats.process_entry(&entry);
                        }

                        let msg = LogMessage {
                            entry,
                            read_at: chrono::Local::now(),
                        };

                        // Отправляем в канал (не блокируясь)
                        if sender.try_send(msg).is_ok() {
                            lines_read.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(LogAnalyzerError::Io(e));
            }
        }
    }

    Ok(())
}

/// Создание канала для связи reader и analyzer
pub fn create_channel(buffer_size: usize) -> (Sender<LogMessage>, Receiver<LogMessage>) {
    bounded(buffer_size)
}

/// Фильтр для логов
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// Фильтр по уровню
    pub levels: Vec<String>,
    /// Ключевое слово для поиска
    pub keyword: Option<String>,
    /// Фильтр по источнику
    pub source: Option<String>,
}

impl LogFilter {
    /// Создание фильтра из строки аргумента
    pub fn from_arg(arg: &str) -> Self {
        let mut filter = Self::default();
        
        // Парсинг формата: level=ERROR,keyword=error,source=auth
        for part in arg.split(',') {
            if let Some((key, value)) = part.split_once('=') {
                match key.trim() {
                    "level" => filter.levels.push(value.trim().to_uppercase()),
                    "keyword" => filter.keyword = Some(value.trim().to_string()),
                    "source" => filter.source = Some(value.trim().to_string()),
                    _ => {}
                }
            }
        }
        
        filter
    }

    /// Проверка соответствия записи фильтру
    pub fn matches(&self, entry: &LogEntry) -> bool {
        // Проверка уровня
        if !self.levels.is_empty() {
            let entry_level = entry.level.as_str().to_uppercase();
            if !self.levels.iter().any(|l| l.to_uppercase() == entry_level) {
                return false;
            }
        }
        
        // Проверка ключевого слова
        if let Some(ref keyword) = self.keyword {
            if !entry.message.to_lowercase().contains(&keyword.to_lowercase()) {
                return false;
            }
        }
        
        // Проверка источника
        if let Some(ref source) = self.source {
            if let Some(ref entry_source) = entry.source {
                if !entry_source.to_lowercase().contains(&source.to_lowercase()) {
                    return false;
                }
            } else {
                return false;
            }
        }
        
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_parser::LogLevel;

    #[test]
    fn test_log_filter_default() {
        let filter = LogFilter::default();
        assert!(filter.levels.is_empty());
        assert!(filter.keyword.is_none());
        assert!(filter.source.is_none());
    }

    #[test]
    fn test_log_filter_from_arg() {
        let filter = LogFilter::from_arg("level=ERROR,keyword=test,source=auth");
        
        assert_eq!(filter.levels, vec!["ERROR"]);
        assert_eq!(filter.keyword, Some("test".to_string()));
        assert_eq!(filter.source, Some("auth".to_string()));
    }

    #[test]
    fn test_log_filter_matches() {
        let filter = LogFilter {
            levels: vec!["ERROR".to_string()],
            keyword: Some("failed".to_string()),
            source: None,
        };
        
        let entry = crate::log_parser::LogEntry {
            timestamp: chrono::Local::now(),
            level: LogLevel::Error,
            message: "Connection failed".to_string(),
            source: None,
            params: std::collections::HashMap::new(),
            file_path: "/test.log".to_string(),
        };
        
        assert!(filter.matches(&entry));
    }

    #[test]
    fn test_log_filter_no_match_level() {
        let filter = LogFilter {
            levels: vec!["ERROR".to_string()],
            keyword: None,
            source: None,
        };
        
        let entry = crate::log_parser::LogEntry {
            timestamp: chrono::Local::now(),
            level: LogLevel::Info,
            message: "Test".to_string(),
            source: None,
            params: std::collections::HashMap::new(),
            file_path: "/test.log".to_string(),
        };
        
        assert!(!filter.matches(&entry));
    }

    #[test]
    fn test_log_filter_no_match_keyword() {
        let filter = LogFilter {
            levels: vec![],
            keyword: Some("notfound".to_string()),
            source: None,
        };
        
        let entry = crate::log_parser::LogEntry {
            timestamp: chrono::Local::now(),
            level: LogLevel::Info,
            message: "Test message".to_string(),
            source: None,
            params: std::collections::HashMap::new(),
            file_path: "/test.log".to_string(),
        };
        
        assert!(!filter.matches(&entry));
    }
}
