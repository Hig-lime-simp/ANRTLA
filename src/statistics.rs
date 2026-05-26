//! Модуль статистики для анализатора логов
//! Потокобезопасная агрегация данных

use crate::log_parser::{LogEntry, LogLevel};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::Local;

/// Основная структура статистики
#[derive(Debug, Clone, Default)]
pub struct Statistics {
    /// Общее количество обработанных строк
    pub total_entries: u64,
    /// Количество сообщений по уровням
    pub entries_by_level: HashMap<String, u64>,
    /// Количество событий по источникам
    pub entries_by_source: HashMap<String, u64>,
    /// Количество ошибок
    pub error_count: u64,
    /// Количество предупреждений
    pub warning_count: u64,
    /// Время начала сбора статистики
    pub started_at: Option<chrono::DateTime<Local>>,
    /// Последнее обновление
    pub last_updated: Option<chrono::DateTime<Local>>,
}

impl Statistics {
    /// Создание новой структуры статистики
    pub fn new() -> Self {
        Self {
            started_at: Some(Local::now()),
            ..Default::default()
        }
    }

    /// Обработка одной записи лога
    pub fn process_entry(&mut self, entry: &LogEntry) {
        self.total_entries += 1;
        self.last_updated = Some(Local::now());
        
        // Подсчёт по уровням
        let level_str = entry.level.as_str().to_string();
        *self.entries_by_level.entry(level_str).or_insert(0) += 1;
        
        // Подсчёт ошибок и предупреждений
        match entry.level {
            LogLevel::Error | LogLevel::Critical => {
                self.error_count += 1;
            }
            LogLevel::Warning => {
                self.warning_count += 1;
            }
            _ => {}
        }
        
        // Подсчёт по источникам
        if let Some(ref source) = entry.source {
            *self.entries_by_source.entry(source.clone()).or_insert(0) += 1;
        }
    }

    /// Формирование краткой сводки
    pub fn summary(&self) -> String {
        format!(
            "Total: {} | Errors: {} | Warnings: {} | Sources: {}",
            self.total_entries,
            self.error_count,
            self.warning_count,
            self.entries_by_source.len()
        )
    }

    /// Формирование подробного отчёта
    pub fn detailed_report(&self) -> String {
        let mut report = String::new();
        report.push_str("=== STATISTICS REPORT ===\n\n");
        
        report.push_str(&format!("Total entries: {}\n", self.total_entries));
        report.push_str(&format!("Errors: {}\n", self.error_count));
        report.push_str(&format!("Warnings: {}\n", self.warning_count));
        
        report.push_str("\n--- By Level ---\n");
        for (level, count) in &self.entries_by_level {
            report.push_str(&format!("  {}: {}\n", level, count));
        }
        
        report.push_str("\n--- By Source ---\n");
        for (source, count) in &self.entries_by_source {
            report.push_str(&format!("  {}: {}\n", source, count));
        }
        
        if let Some(started) = self.started_at {
            report.push_str(&format!("\nStarted at: {}\n", started.format("%Y-%m-%d %H:%M:%S")));
        }
        
        report
    }
}

/// Потокобезопасная обёртка для статистики
#[derive(Debug, Clone)]
pub struct SharedStatistics {
    inner: Arc<Mutex<Statistics>>,
    /// Детектор аномалий — вызывается для каждой строки прямо в Reader
    anomaly_detector: Option<SharedAnomalyDetector>,
    /// Накопленные аномалии — shared с main для live-вывода
    anomalies: Arc<Mutex<Vec<Anomaly>>>,
}

impl SharedStatistics {
    /// Создание новой потокобезопасной статистики
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Statistics::new())),
            anomaly_detector: None,
            anomalies: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Подключить детектор аномалий (вызвать до запуска Reader)
    pub fn set_anomaly_detector(&mut self, threshold: u32, window_seconds: u64) {
        self.anomaly_detector = Some(SharedAnomalyDetector::new(threshold, window_seconds));
    }

    /// Обработка записи из любого потока — также запускает детектор аномалий
    pub fn process_entry(&self, entry: &LogEntry) {
        if let Ok(mut stats) = self.inner.lock() {
            stats.process_entry(entry);
        }
        if let Some(ref detector) = self.anomaly_detector {
            let detected = detector.check(entry);
            if !detected.is_empty() {
                if let Ok(mut lock) = self.anomalies.lock() {
                    lock.extend(detected);
                }
            }
        }
    }

    /// Получение копии текущей статистики
    pub fn get_stats(&self) -> Statistics {
        self.inner
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Arc на список аномалий — для live-вывода и финального отчёта
    pub fn get_anomalies_arc(&self) -> Arc<Mutex<Vec<Anomaly>>> {
        Arc::clone(&self.anomalies)
    }

    /// Получение Arc ссылки для передачи в воркеры
    pub fn clone_arc(&self) -> Arc<Mutex<Statistics>> {
        Arc::clone(&self.inner)
    }
}

impl Default for SharedStatistics {
    fn default() -> Self {
        Self::new()
    }
}

/// Детектор аномалий в потоке логов
#[derive(Debug)]
pub struct AnomalyDetector {
    /// Порог количества одинаковых сообщений в окне
    threshold: u32,
    /// Окно времени для анализа (секунды)
    window_seconds: u64,
    /// История сообщений для обнаружения повторений
    message_history: HashMap<String, Vec<chrono::DateTime<Local>>>,
    /// Временные метки ошибок для ErrorSpike
    error_timestamps: Vec<chrono::DateTime<Local>>,
    /// Временные метки всех сообщений для MessageFlood
    all_timestamps: Vec<chrono::DateTime<Local>>,
    /// Последние сгенерированные предупреждения
    pub anomalies: Vec<Anomaly>,
}

/// Структура обнаруженной аномалии
#[derive(Debug, Clone)]
pub struct Anomaly {
    /// Тип аномалии
    pub anomaly_type: AnomalyType,
    /// Описание
    pub description: String,
    /// Время обнаружения
    pub detected_at: chrono::DateTime<Local>,
    /// Связанное сообщение
    pub message: Option<String>,
}

/// Типы аномалий
#[derive(Debug, Clone, PartialEq)]
pub enum AnomalyType {
    /// Превышение порога ошибок
    ErrorSpike,
    /// Повторяющиеся сообщения
    RepeatedMessage,
    /// Резкий рост количества сообщений
    MessageFlood,
    /// Один тип сообщений постоянно
    SingleMessageType,
}

impl AnomalyDetector {
    /// Создание детектора с заданным порогом
    pub fn new(threshold: u32, window_seconds: u64) -> Self {
        Self {
            threshold,
            window_seconds,
            message_history: HashMap::new(),
            error_timestamps: Vec::new(),
            all_timestamps: Vec::new(),
            anomalies: Vec::new(),
        }
    }

    /// Проверка записи на аномалии — возвращает все обнаруженные аномалии
    pub fn check(&mut self, entry: &LogEntry) -> Vec<Anomaly> {
        let now = Local::now();
        let cutoff = now - chrono::Duration::seconds(self.window_seconds as i64);
        let mut found = Vec::new();

        // 1. RepeatedMessage — одно и то же сообщение повторяется threshold раз в окне
        let msg_key = format!("{}:{}", entry.level.as_str(), entry.message);
        let timestamps = self.message_history.entry(msg_key).or_default();
        timestamps.push(now);
        timestamps.retain(|&t| t > cutoff);
        if timestamps.len() >= self.threshold as usize {
            let anomaly = Anomaly {
                anomaly_type: AnomalyType::RepeatedMessage,
                description: format!(
                    "Message repeated {} times in {}s: \"{}\"",
                    timestamps.len(), self.window_seconds, entry.message
                ),
                detected_at: now,
                message: Some(entry.message.clone()),
            };
            self.anomalies.push(anomaly.clone());
            timestamps.clear();
            found.push(anomaly);
        }

        // 2. ErrorSpike — threshold ошибок/критических записей в окне
        if matches!(entry.level, LogLevel::Error | LogLevel::Critical) {
            self.error_timestamps.push(now);
            self.error_timestamps.retain(|&t| t > cutoff);
            if self.error_timestamps.len() >= self.threshold as usize {
                let anomaly = Anomaly {
                    anomaly_type: AnomalyType::ErrorSpike,
                    description: format!(
                        "{} errors in {}s",
                        self.error_timestamps.len(), self.window_seconds
                    ),
                    detected_at: now,
                    message: Some(entry.message.clone()),
                };
                self.anomalies.push(anomaly.clone());
                self.error_timestamps.clear();
                found.push(anomaly);
            }
        }

        // 3. MessageFlood — любых сообщений > threshold * 10 в окне
        self.all_timestamps.push(now);
        self.all_timestamps.retain(|&t| t > cutoff);
        let flood_threshold = self.threshold as usize * 10;
        if self.all_timestamps.len() >= flood_threshold {
            let anomaly = Anomaly {
                anomaly_type: AnomalyType::MessageFlood,
                description: format!(
                    "{} messages in {}s (threshold: {})",
                    self.all_timestamps.len(), self.window_seconds, flood_threshold
                ),
                detected_at: now,
                message: None,
            };
            self.anomalies.push(anomaly.clone());
            self.all_timestamps.clear();
            found.push(anomaly);
        }

        found
    }

    /// Проверка на поток одного типа сообщений
    pub fn check_single_message_type(&self, entries: &[LogEntry]) -> Option<Anomaly> {
        if entries.is_empty() {
            return None;
        }
        
        let first_level = &entries[0].level;
        let all_same = entries.iter().all(|e| e.level == *first_level);
        
        if all_same && entries.len() > 10 {
            Some(Anomaly {
                anomaly_type: AnomalyType::SingleMessageType,
                description: format!("All {} messages are of type {:?}", entries.len(), first_level),
                detected_at: Local::now(),
                message: None,
            })
        } else {
            None
        }
    }

    /// Получение всех обнаруженных аномалий
    pub fn get_anomalies(&self) -> &[Anomaly] {
        &self.anomalies
    }

    /// Очистка истории аномалий
    pub fn clear_anomalies(&mut self) {
        self.anomalies.clear();
    }
}

/// Потокобезопасная обёртка для детектора аномалий
#[derive(Debug, Clone)]
pub struct SharedAnomalyDetector {
    inner: Arc<Mutex<AnomalyDetector>>,
}

impl SharedAnomalyDetector {
    pub fn new(threshold: u32, window_seconds: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AnomalyDetector::new(threshold, window_seconds))),
        }
    }

    /// Проверяет запись — возвращает все обнаруженные аномалии
    pub fn check(&self, entry: &LogEntry) -> Vec<Anomaly> {
        self.inner.lock().map(|mut d| d.check(entry)).unwrap_or_default()
    }

    /// Возвращает все накопленные аномалии
    pub fn get_all(&self) -> Vec<Anomaly> {
        self.inner.lock().map(|d| d.anomalies.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_parser::quick_parse;

    #[test]
    fn test_statistics_new() {
        let stats = Statistics::new();
        assert_eq!(stats.total_entries, 0);
        assert!(stats.started_at.is_some());
    }

    #[test]
    fn test_process_entry() {
        let mut stats = Statistics::new();
        let entry = quick_parse("INFO Test message", "/test.log").unwrap();
        
        stats.process_entry(&entry);
        
        assert_eq!(stats.total_entries, 1);
        assert_eq!(stats.entries_by_level.get("INFO"), Some(&1));
    }

    #[test]
    fn test_process_error_entry() {
        let mut stats = Statistics::new();
        let entry = quick_parse("ERROR Something failed", "/test.log").unwrap();
        
        stats.process_entry(&entry);
        
        assert_eq!(stats.error_count, 1);
        assert_eq!(stats.warning_count, 0);
    }

    #[test]
    fn test_shared_statistics() {
        let shared = SharedStatistics::new();
        let entry = quick_parse("INFO Test", "/test.log").unwrap();
        
        shared.process_entry(&entry);
        
        let stats = shared.get_stats();
        assert_eq!(stats.total_entries, 1);
    }

    #[test]
    fn test_anomaly_detector_new() {
        let detector = AnomalyDetector::new(20, 1);
        assert_eq!(detector.threshold, 20);
        assert_eq!(detector.window_seconds, 1);
    }

    #[test]
    fn test_anomaly_detection_repeated() {
        let mut detector = AnomalyDetector::new(3, 10);
        let entry = quick_parse("ERROR Same error", "/test.log").unwrap();
        
        // Отправляем одно и то же сообщение 3 раза
        for _ in 0..3 {
            detector.check(&entry);
        }
        
        // Должна быть обнаружена аномалия
        assert!(!detector.get_anomalies().is_empty());
    }

    #[test]
    fn test_statistics_summary() {
        let mut stats = Statistics::new();
        stats.process_entry(&quick_parse("INFO Test", "/test.log").unwrap());
        stats.process_entry(&quick_parse("ERROR Fail", "/test.log").unwrap());
        
        let summary = stats.summary();
        assert!(summary.contains("Total: 2"));
        assert!(summary.contains("Errors: 1"));
    }
}
