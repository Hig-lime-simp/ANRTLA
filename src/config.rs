//! Модуль конфигурации для анализатора логов
//! Поддерживает JSON, YAML, TOML форматы

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Основная структура конфигурации
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Правила анализа логов
    pub rules: Vec<Rule>,
    /// Настройки приложения
    #[serde(default)]
    pub settings: Settings,
}

/// Правило для анализа логов
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    /// Имя правила
    pub name: String,
    /// Паттерн для поиска (regex или ключевое слово)
    pub pattern: String,
    /// Тип действия при совпадении
    #[serde(default = "default_action")]
    pub action: ActionType,
    /// Уровень важности
    #[serde(default = "default_severity")]
    pub severity: Severity,
    /// Включено ли правило
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Порог срабатывания (для аномалий)
    #[serde(default)]
    pub threshold: Option<u32>,
    /// Окно времени в секундах (для аномалий)
    #[serde(default)]
    pub time_window: Option<u64>,
}

/// Тип действия при совпадении правила
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    /// Просто подсчитать
    Count,
    /// Выдать предупреждение
    Warn,
    /// Игнорировать
    Ignore,
    /// Записать в отчёт
    Report,
}

/// Уровень важности события
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Настройки приложения
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Размер буфера для чтения
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    /// Формат вывода
    #[serde(default)]
    pub output_format: OutputFormat,
    /// Количество воркеров
    #[serde(default = "default_workers")]
    pub workers: u32,
    /// Путь для сохранения отчётов
    #[serde(default)]
    pub report_dir: Option<String>,
    /// Порог для обнаружения аномалий (сообщений в секунду)
    #[serde(default = "default_anomaly_threshold")]
    pub anomaly_threshold: u32,
}

fn default_action() -> ActionType {
    ActionType::Count
}

fn default_severity() -> Severity {
    Severity::Info
}

fn default_enabled() -> bool {
    true
}

fn default_buffer_size() -> usize {
    1024
}

fn default_workers() -> u32 {
    4
}

fn default_anomaly_threshold() -> u32 {
    20
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Pretty,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            settings: Settings::default(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            buffer_size: default_buffer_size(),
            output_format: OutputFormat::default(),
            workers: default_workers(),
            report_dir: None,
            anomaly_threshold: default_anomaly_threshold(),
        }
    }
}

/// Загрузка и валидация конфигурации из файла
pub fn load_and_validate(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(&content)?,
        Some("yaml" | "yml") => serde_yaml::from_str(&content)?,
        Some("toml") => toml::from_str(&content)?,
        _ => serde_json::from_str(&content)?,
    };
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(config.rules.is_empty());
        assert_eq!(config.settings.buffer_size, 1024);
        assert_eq!(config.settings.workers, 4);
    }

    #[test]
    fn test_rule_creation() {
        let rule = Rule {
            name: "test_rule".to_string(),
            pattern: "error".to_string(),
            action: ActionType::Warn,
            severity: Severity::Error,
            enabled: true,
            threshold: None,
            time_window: None,
        };
        
        assert_eq!(rule.name, "test_rule");
        assert_eq!(rule.action, ActionType::Warn);
    }

    #[test]
    fn test_action_type_equality() {
        assert_eq!(ActionType::Count, ActionType::Count);
        assert_ne!(ActionType::Count, ActionType::Warn);
    }

    #[test]
    fn test_config_json_deserialization() {
        let json = r#"{
            "rules": [
                {
                    "name": "error_detector",
                    "pattern": "ERROR",
                    "action": "warn",
                    "severity": "error"
                }
            ],
            "settings": {
                "workers": 8
            }
        }"#;
        
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].name, "error_detector");
        assert_eq!(config.settings.workers, 8);
    }
}
