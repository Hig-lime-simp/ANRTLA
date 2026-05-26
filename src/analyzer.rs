//! Модуль анализатора логов (Analyzer)
//! Обрабатывает распарсенные записи, применяет правила и обнаруживает аномалии

use crate::config::{Config, ActionType};
use crate::error::Result;
use crate::log_parser::LogEntry;
use crate::reader::LogMessage;
use crate::statistics::{SharedStatistics, Statistics};
use crossbeam_channel::Receiver;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Результат применения правила
#[derive(Debug, Clone)]
pub struct RuleMatch {
    /// Имя правила
    pub rule_name: String,
    /// Тип действия
    pub action: ActionType,
    /// Запись лога
    pub entry: LogEntry,
}

/// Аналайзер для обработки логов
pub struct Analyzer {
    /// Конфигурация с правилами
    config: Config,
    /// Общая статистика (содержит детектор аномалий)
    statistics: SharedStatistics,
    /// Канал для получения сообщений
    receiver: Receiver<LogMessage>,
    /// Флаг остановки
    running: Arc<std::sync::atomic::AtomicBool>,
    /// Применённые правила
    matched_rules: Vec<RuleMatch>,
}

impl Analyzer {
    /// Создание нового анализатора
    pub fn new(
        config: Config,
        statistics: SharedStatistics,
        receiver: Receiver<LogMessage>,
    ) -> Self {
        Self {
            config,
            statistics,
            receiver,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            matched_rules: Vec::new(),
        }
    }

    /// Запуск анализа в отдельном потоке
    pub fn start(&mut self) -> JoinHandle<Result<()>> {
        use std::sync::atomic::Ordering;

        let running = Arc::clone(&self.running);
        let receiver = self.receiver.clone();
        let rules = self.config.rules.clone();

        tokio::spawn(async move {
            running.store(true, Ordering::Relaxed);

            while running.load(Ordering::Relaxed) {
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(msg) => {
                        for rule in &rules {
                            if !rule.enabled {
                                continue;
                            }
                            if let Ok(regex) = regex::Regex::new(&rule.pattern) {
                                if regex.is_match(&msg.entry.message) {
                                    log::debug!("Rule '{}' matched: {}", rule.name, msg.entry.message);
                                }
                            }
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        log::info!("Channel disconnected, stopping analyzer");
                        break;
                    }
                }
            }

            Ok(())
        })
    }

    /// Остановка анализатора
    pub fn stop(&self) {
        use std::sync::atomic::Ordering;
        self.running.store(false, Ordering::Relaxed);
    }

    /// Проверка статуса работы
    pub fn is_running(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.running.load(Ordering::Relaxed)
    }

    /// Применение правил к записи
    pub fn apply_rules(&mut self, entry: &LogEntry) -> Vec<RuleMatch> {
        let mut matches = Vec::new();
        
        for rule in &self.config.rules {
            if !rule.enabled {
                continue;
            }
            
            // Проверяем паттерн
            let matched = if let Ok(regex) = regex::Regex::new(&rule.pattern) {
                regex.is_match(&entry.message)
            } else {
                // Если regex не компилируется, пробуем простое совпадение
                entry.message.contains(&rule.pattern)
            };
            
            if matched {
                matches.push(RuleMatch {
                    rule_name: rule.name.clone(),
                    action: rule.action.clone(),
                    entry: entry.clone(),
                });
                
                // Выполняем действие
                match rule.action {
                    ActionType::Ignore => {
                        // Игнорируем запись
                    }
                    ActionType::Warn => {
                        log::warn!("[{}] {}", rule.name, entry.message);
                    }
                    ActionType::Report => {
                        self.matched_rules.push(matches.last().unwrap().clone());
                    }
                    ActionType::Count => {
                        // Просто считаем
                    }
                }
            }
        }
        
        matches
    }

    /// Получение статистики
    pub fn get_statistics(&self) -> Statistics {
        self.statistics.get_stats()
    }

    /// Получение применённых правил
    pub fn get_matched_rules(&self) -> &[RuleMatch] {
        &self.matched_rules
    }
}

/// Воркер для параллельной обработки логов
pub struct Worker {
    /// ID воркера
    id: usize,
    /// Конфигурация
    config: Arc<Config>,
    /// Статистика
    statistics: SharedStatistics,
}

impl Worker {
    /// Создание нового воркера
    pub fn new(id: usize, config: Arc<Config>, statistics: SharedStatistics) -> Self {
        Self {
            id,
            config,
            statistics,
        }
    }

    /// Обработка одной записи
    pub fn process(&self, entry: &LogEntry) -> Vec<RuleMatch> {
        let mut matches = Vec::new();

        // Применяем правила (статистика обновляется в Reader)
        for rule in &self.config.rules {
            if !rule.enabled {
                continue;
            }
            
            let matched = if let Ok(regex) = regex::Regex::new(&rule.pattern) {
                regex.is_match(&entry.message)
            } else {
                entry.message.contains(&rule.pattern)
            };
            
            if matched {
                matches.push(RuleMatch {
                    rule_name: rule.name.clone(),
                    action: rule.action.clone(),
                    entry: entry.clone(),
                });
            }
        }
        
        matches
    }

    /// Получение ID воркера
    pub fn id(&self) -> usize {
        self.id
    }
}

/// Менеджер воркеров для параллельной обработки
pub struct WorkerManager {
    /// Количество воркеров
    worker_count: usize,
    /// Конфигурация
    config: Arc<Config>,
    /// Общая статистика
    statistics: SharedStatistics,
}

impl WorkerManager {
    /// Создание менеджера воркеров
    pub fn new(worker_count: usize, config: Config, statistics: SharedStatistics) -> Self {
        Self {
            worker_count,
            config: Arc::new(config),
            statistics,
        }
    }

    /// Запуск воркеров
    pub fn spawn_workers(
        &self,
        receiver: Receiver<LogMessage>,
    ) -> Vec<JoinHandle<Result<()>>> {
        let mut handles = Vec::new();
        
        for i in 0..self.worker_count {
            let config = Arc::clone(&self.config);
            let stats = self.statistics.clone();
            let rx = receiver.clone();
            
            let handle = tokio::spawn(async move {
                let worker = Worker::new(i, config, stats);
                
                loop {
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(msg) => {
                            let _matches = worker.process(&msg.entry);
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            continue;
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                            break;
                        }
                    }
                }
                
                Ok(())
            });
            
            handles.push(handle);
        }
        
        handles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Rule, Severity};
    use crate::log_parser::LogLevel;
    use crate::reader::create_channel;

    #[test]
    fn test_rule_match_creation() {
        use crate::log_parser::quick_parse;
        
        let entry = quick_parse("ERROR Test error", "/test.log").unwrap();
        let rule_match = RuleMatch {
            rule_name: "test_rule".to_string(),
            action: ActionType::Warn,
            entry,
        };
        
        assert_eq!(rule_match.rule_name, "test_rule");
        assert_eq!(rule_match.action, ActionType::Warn);
    }

    #[test]
    fn test_analyzer_new() {
        let config = Config::default();
        let stats = SharedStatistics::new();
        let (_, rx) = create_channel(100);
        
        let analyzer = Analyzer::new(config, stats, rx);
        
        assert!(!analyzer.is_running());
    }

    #[test]
    fn test_worker_process() {
        use crate::log_parser::quick_parse;

        let config = Arc::new(Config {
            rules: vec![Rule {
                name: "error_rule".to_string(),
                pattern: "failed".to_string(), // матчит message, а не уровень
                action: ActionType::Count,
                severity: Severity::Error,
                enabled: true,
                threshold: None,
                time_window: None,
            }],
            settings: Default::default(),
        });

        let stats = SharedStatistics::new();
        let worker = Worker::new(0, config, stats);

        let entry = quick_parse("ERROR Something failed", "/test.log").unwrap();
        let matches = worker.process(&entry);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].rule_name, "error_rule");
    }

    #[test]
    fn test_apply_rules_with_matching() {
        let config = Config {
            rules: vec![Rule {
                name: "test".to_string(),
                pattern: "error".to_string(),
                action: ActionType::Count,
                severity: Severity::Error,
                enabled: true,
                threshold: None,
                time_window: None,
            }],
            settings: Default::default(),
        };
        
        let stats = SharedStatistics::new();
        let (_, rx) = create_channel(100);
        let mut analyzer = Analyzer::new(config, stats, rx);
        
        let entry = crate::log_parser::LogEntry {
            timestamp: chrono::Local::now(),
            level: LogLevel::Error,
            message: "This is an error".to_string(),
            source: None,
            params: std::collections::HashMap::new(),
            file_path: "/test.log".to_string(),
        };
        
        let matches = analyzer.apply_rules(&entry);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_apply_rules_no_match() {
        let config = Config {
            rules: vec![Rule {
                name: "test".to_string(),
                pattern: "notfound".to_string(),
                action: ActionType::Count,
                severity: Severity::Info,
                enabled: true,
                threshold: None,
                time_window: None,
            }],
            settings: Default::default(),
        };
        
        let stats = SharedStatistics::new();
        let (_, rx) = create_channel(100);
        let mut analyzer = Analyzer::new(config, stats, rx);
        
        let entry = crate::log_parser::LogEntry {
            timestamp: chrono::Local::now(),
            level: LogLevel::Info,
            message: "Normal message".to_string(),
            source: None,
            params: std::collections::HashMap::new(),
            file_path: "/test.log".to_string(),
        };
        
        let matches = analyzer.apply_rules(&entry);
        assert!(matches.is_empty());
    }
}
