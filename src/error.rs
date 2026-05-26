//! Модуль ошибок для анализатора логов
//! Предоставляет типизированные ошибки для различных сценариев

use thiserror::Error;

/// Основные типы ошибок приложения
#[derive(Error, Debug)]
pub enum LogAnalyzerError {
    /// Ошибка ввода-вывода
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Ошибка конфигурации
    #[error("Configuration error: {0}")]
    Config(String),

    /// Ошибка парсинга лога
    #[error("Parse error: {0}")]
    Parse(String),

    /// Ошибка сессии
    #[error("Session error: {0}")]
    Session(String),

    /// Ошибка анализа
    #[error("Analysis error: {0}")]
    Analysis(String),

    /// Ошибка канала передачи данных
    #[error("Channel error: {0}")]
    Channel(String),

    /// Ошибка anyhow
    #[error("Anyhow error: {0}")]
    Anyhow(#[from] anyhow::Error),
}

/// Тип Result для нашего приложения
pub type Result<T> = std::result::Result<T, LogAnalyzerError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err: LogAnalyzerError = io_err.into();
        
        match err {
            LogAnalyzerError::Io(e) => {
                assert_eq!(e.kind(), io::ErrorKind::NotFound);
            }
            _ => panic!("Expected Io error"),
        }
    }

    #[test]
    fn test_config_error() {
        let err = LogAnalyzerError::Config("Invalid config".to_string());
        assert!(err.to_string().contains("Invalid config"));
    }

    #[test]
    fn test_parse_error() {
        let err = LogAnalyzerError::Parse("Failed to parse".to_string());
        assert!(err.to_string().contains("Failed to parse"));
    }

    #[test]
    fn test_session_error() {
        let err = LogAnalyzerError::Session("Session not found".to_string());
        assert!(err.to_string().contains("Session not found"));
    }

    #[test]
    fn test_analysis_error() {
        let err = LogAnalyzerError::Analysis("Analysis failed".to_string());
        assert!(err.to_string().contains("Analysis failed"));
    }

    #[test]
    fn test_channel_error() {
        let err = LogAnalyzerError::Channel("Channel disconnected".to_string());
        assert!(err.to_string().contains("Channel disconnected"));
    }
}
