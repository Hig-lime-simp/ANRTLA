//! Модуль парсинга логов
//! Поддерживает базовый формат: timestamp LEVEL message [параметры]

use crate::error::{LogAnalyzerError, Result};
use chrono::{DateTime, Local};
use regex::Regex;
use std::collections::HashMap;

/// Структура записи лога
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// Временная метка
    pub timestamp: DateTime<Local>,
    /// Уровень события
    pub level: LogLevel,
    /// Текст сообщения
    pub message: String,
    /// Источник события (service, module)
    pub source: Option<String>,
    /// Дополнительные параметры
    pub params: HashMap<String, String>,
    /// Путь к файлу источника
    pub file_path: String,
}

/// Уровни логирования
#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
    Unknown(String),
}

impl From<&str> for LogLevel {
    fn from(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "DEBUG" | "DBG" => LogLevel::Debug,
            "INFO" | "INF" => LogLevel::Info,
            "WARN" | "WARNING" | "WRN" => LogLevel::Warning,
            "ERROR" | "ERR" => LogLevel::Error,
            "CRITICAL" | "FATAL" | "CRT" | "CRIT" => LogLevel::Critical,
            _ => LogLevel::Unknown(s.to_string()),
        }
    }
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARNING",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRITICAL",
            LogLevel::Unknown(_) => "UNKNOWN",
        }
    }
}

/// Парсер для разбора строк логов
pub struct LogParser {
    /// Regex паттерн для парсинга
    pattern: Regex,
}

impl LogParser {
    /// Создание нового парсера с паттерном по умолчанию
    pub fn new() -> Self {
        // Паттерн: timestamp LEVEL message [key=value ...]
        let pattern = Regex::new(
            r"^(?P<timestamp>\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)?\s*(?P<level>[A-Z]{3,10})\s+(?P<message>.+?)\s*(?:\[(?P<params>[^\]]*)\])?$"
        ).unwrap_or_else(|_| Regex::new(r".*").unwrap());
        
        Self { pattern }
    }

    /// Парсинг одной строки лога
    pub fn parse_line(&self, line: &str, file_path: &str) -> Result<LogEntry> {
        let line = line.trim();
        if line.is_empty() {
            return Err(LogAnalyzerError::Parse("Empty line".to_string()));
        }

        // Паттерн для даты: YYYY-MM-DD HH:MM:SS или YYYY-MM-DDTHH:MM:SS
        let date_pattern = Regex::new(r"^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}\s*").unwrap();
        
        // Удаляем временную метку если она есть
        let line_without_timestamp = date_pattern.replace(line, "");
        let line_to_parse = line_without_timestamp.trim();
        
        // Простой парсинг для формата: LEVEL message
        let parts: Vec<&str> = line_to_parse.splitn(2, |c: char| c.is_whitespace()).collect();
        
        let (level, message) = if parts.len() >= 2 {
            // Нормализуем уровень лога (приводим к верхнему регистру для сравнения)
            let level_str = parts[0];
            let level = match level_str.to_uppercase().as_str() {
                "DEBUG" | "DBG" => LogLevel::Debug,
                "INFO" | "INF" => LogLevel::Info,
                "WARN" | "WARNING" | "WRN" => LogLevel::Warning,
                "ERROR" | "ERR" => LogLevel::Error,
                "CRITICAL" | "FATAL" | "CRT" | "CRIT" => LogLevel::Critical,
                _ => LogLevel::Unknown(level_str.to_string()),
            };
            (level, parts[1].to_string())
        } else {
            let level_str = line_to_parse;
            let level = match level_str.to_uppercase().as_str() {
                "DEBUG" | "DBG" => LogLevel::Debug,
                "INFO" | "INF" => LogLevel::Info,
                "WARN" | "WARNING" | "WRN" => LogLevel::Warning,
                "ERROR" | "ERR" => LogLevel::Error,
                "CRITICAL" | "FATAL" | "CRT" | "CRIT" => LogLevel::Critical,
                _ => LogLevel::Unknown(level_str.to_string()),
            };
            (level, level_str.to_string())
        };

        // Извлечение параметров из сообщения
        let (clean_message, params) = extract_params(&message);
        
        // Извлечение источника если указан в формате service=name
        let source = params.get("service").cloned()
            .or_else(|| params.get("source").cloned())
            .or_else(|| {
                // Если источник не указан в параметрах, используем имя файла как источник
                std::path::Path::new(file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            });

        Ok(LogEntry {
            timestamp: Local::now(),
            level,
            message: clean_message,
            source,
            params,
            file_path: file_path.to_string(),
        })
    }

    /// Парсинг с кастомным форматом
    pub fn parse_with_format(&self, line: &str, _format: &str, file_path: &str) -> Result<LogEntry> {
        // Пока используем стандартный парсинг
        // В будущем можно добавить поддержку различных форматов
        self.parse_line(line, file_path)
    }
}

impl Default for LogParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Вспомогательная функция для разделения по пробельным символам
fn whitespace(c: char) -> bool {
    c.is_whitespace()
}

/// Извлечение параметров из сообщения в формате key=value
fn extract_params(message: &str) -> (String, HashMap<String, String>) {
    let mut params: HashMap<String, String> = HashMap::new();
    
    // Ищем паттерны key=value, исключая ] из значений
    let param_regex = Regex::new(r"(\w+)=([^\s\]]+)").unwrap();
    
    for cap in param_regex.captures_iter(message) {
        if let (Some(key), Some(value)) = (cap.get(1), cap.get(2)) {
            params.insert(key.as_str().to_string(), value.as_str().to_string());
        }
    }
    
    // Удаляем параметры из сообщения
    let clean_message = param_regex.replace_all(message, "").to_string();
    
    (clean_message.trim().to_string(), params)
}

/// Быстрый парсер для простых случаев
pub fn quick_parse(line: &str, file_path: &str) -> Option<LogEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    // Паттерн для даты: YYYY-MM-DD HH:MM:SS или YYYY-MM-DDTHH:MM:SS
    let date_pattern = Regex::new(r"^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}\s*").unwrap();
    
    // Удаляем временную метку если она есть
    let line_to_parse = date_pattern.replace(line, "").trim().to_string();
    
    let parts: Vec<&str> = line_to_parse.splitn(2, |c: char| c.is_whitespace()).collect();
    
    let (level, message) = if parts.len() >= 2 {
        // Нормализуем уровень лога
        let level_str = parts[0];
        let level = match level_str.to_uppercase().as_str() {
            "DEBUG" | "DBG" => LogLevel::Debug,
            "INFO" | "INF" => LogLevel::Info,
            "WARN" | "WARNING" | "WRN" => LogLevel::Warning,
            "ERROR" | "ERR" => LogLevel::Error,
            "CRITICAL" | "FATAL" | "CRT" | "CRIT" => LogLevel::Critical,
            _ => LogLevel::Unknown(level_str.to_string()),
        };
        (level, parts[1].to_string())
    } else {
        let level_str = line_to_parse.clone();
        let level = match level_str.to_uppercase().as_str() {
            "DEBUG" | "DBG" => LogLevel::Debug,
            "INFO" | "INF" => LogLevel::Info,
            "WARN" | "WARNING" | "WRN" => LogLevel::Warning,
            "ERROR" | "ERR" => LogLevel::Error,
            "CRITICAL" | "FATAL" | "CRT" | "CRIT" => LogLevel::Critical,
            _ => LogLevel::Unknown(level_str.to_string()),
        };
        (level, line_to_parse)
    };

    let (_, params) = extract_params(&message);
    let source = params.get("service").cloned()
        .or_else(|| params.get("source").cloned())
        .or_else(|| {
            // Если источник не указан в параметрах, используем имя файла как источник
            std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        });

    Some(LogEntry {
        timestamp: Local::now(),
        level,
        message,
        source,
        params,
        file_path: file_path.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_from_str() {
        assert_eq!(LogLevel::from("INFO"), LogLevel::Info);
        assert_eq!(LogLevel::from("info"), LogLevel::Info);
        assert_eq!(LogLevel::from("ERROR"), LogLevel::Error);
        assert_eq!(LogLevel::from("warn"), LogLevel::Warning);
        assert_eq!(LogLevel::from("WARNING"), LogLevel::Warning);
        assert_eq!(LogLevel::from("CRITICAL"), LogLevel::Critical);
        assert_eq!(LogLevel::from("DEBUG"), LogLevel::Debug);
    }

    #[test]
    fn test_log_level_unknown() {
        let level = LogLevel::from("CUSTOM");
        assert!(matches!(level, LogLevel::Unknown(_)));
    }

    #[test]
    fn test_parser_new() {
        let parser = LogParser::new();
        assert!(parser.pattern.is_match("INFO test message"));
    }

    #[test]
    fn test_parse_simple_line() {
        let parser = LogParser::new();
        let entry = parser.parse_line("INFO User logged in", "/var/log/app.log").unwrap();
        
        assert_eq!(entry.level, LogLevel::Info);
        assert!(entry.message.contains("User logged in"));
    }

    #[test]
    fn test_parse_error_line() {
        let parser = LogParser::new();
        let entry = parser.parse_line("ERROR Database connection failed", "/var/log/app.log").unwrap();
        
        assert_eq!(entry.level, LogLevel::Error);
        assert!(entry.message.contains("Database connection failed"));
    }

    #[test]
    fn test_parse_with_params() {
        let parser = LogParser::new();
        let entry = parser.parse_line("INFO Connection established [service=auth user=admin]", "/var/log/app.log").unwrap();
        
        assert_eq!(entry.params.get("service"), Some(&"auth".to_string()));
        assert_eq!(entry.params.get("user"), Some(&"admin".to_string()));
        assert_eq!(entry.source, Some("auth".to_string()));
    }

    #[test]
    fn test_quick_parse() {
        let entry = quick_parse("WARN Low memory", "/var/log/system.log");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.level, LogLevel::Warning);
    }

    #[test]
    fn test_quick_parse_empty() {
        let entry = quick_parse("", "/var/log/system.log");
        assert!(entry.is_none());
    }

    #[test]
    fn test_extract_params() {
        let (msg, params) = extract_params("Test message [service=web user=john]");
        assert!(msg.contains("Test message"));
        assert_eq!(params.get("service"), Some(&"web".to_string()));
        assert_eq!(params.get("user"), Some(&"john".to_string()));
    }
}
