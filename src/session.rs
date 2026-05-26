//! Модуль управления сессиями
//! Позволяет сохранять и восстанавливать контекст анализа

use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Структура сессии анализа
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Уникальный идентификатор сессии
    pub id: String,
    /// Имя сессии (задаётся пользователем)
    pub name: String,
    /// Время создания
    pub created_at: String,
    /// Список файлов для анализа
    pub log_files: Vec<PathBuf>,
    /// Путь к конфигурационному файлу
    pub config_path: Option<PathBuf>,
    /// Количество воркеров
    pub workers: u32,
    /// Статус сессии
    pub status: SessionStatus,
}

/// Статус сессии
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// Сессия активна
    Active,
    /// Сессия приостановлена
    Paused,
    /// Сессия завершена
    Completed,
    /// Ошибка в сессии
    Error,
}

impl Session {
    /// Создание новой сессии
    pub fn new(name: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            log_files: Vec::new(),
            config_path: None,
            workers: 4,
            status: SessionStatus::Active,
        }
    }

    /// Сохранение сессии на диск
    pub fn save(&self) -> Result<()> {
        let sessions_dir = get_sessions_dir();
        fs::create_dir_all(&sessions_dir)?;
        
        let session_path = sessions_dir.join(format!("{}.json", self.name));
        let content = serde_json::to_string_pretty(self)?;
        fs::write(session_path, content)?;
        Ok(())
    }

    /// Загрузка сессии по имени
    pub fn load(name: &str) -> Result<Option<Self>> {
        let sessions_dir = get_sessions_dir();
        let session_path = sessions_dir.join(format!("{}.json", name));
        
        if session_path.exists() {
            let content = fs::read_to_string(session_path)?;
            let session: Session = serde_json::from_str(&content)?;
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    /// Добавление файла для анализа
    pub fn add_file(&mut self, path: PathBuf) {
        self.log_files.push(path);
    }

    /// Установка конфигурации
    pub fn set_config(&mut self, path: PathBuf) {
        self.config_path = Some(path);
    }

    /// Установка количества воркеров
    pub fn set_workers(&mut self, workers: u32) {
        self.workers = workers;
    }

    /// Обновление статуса
    pub fn set_status(&mut self, status: SessionStatus) {
        self.status = status;
        let _ = self.save();
    }
}

/// Получение директории для хранения сессий
fn get_sessions_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("log_analyzer")
        .join("sessions")
}

/// Список всех доступных сессий
pub fn list_sessions() -> Result<Vec<Session>> {
    let sessions_dir = get_sessions_dir();
    let mut sessions = Vec::new();
    
    if !sessions_dir.exists() {
        return Ok(sessions);
    }

    let entries = fs::read_dir(&sessions_dir)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(session) = serde_json::from_str::<Session>(&content) {
                    sessions.push(session);
                }
            }
        }
    }

    Ok(sessions)
}

/// Удаление сессии
pub fn remove_session(name: &str) -> Result<()> {
    let sessions_dir = get_sessions_dir();
    let session_path = sessions_dir.join(format!("{}.json", name));
    
    if session_path.exists() {
        fs::remove_file(session_path)?;
        Ok(())
    } else {
        anyhow::bail!("Session '{}' not found", name)
    }
}

/// Просмотр статистики сессии
pub fn view_session_info(name: &str) -> Result<String> {
    match Session::load(name)? {
        Some(session) => {
            let mut info = String::new();
            info.push_str(&format!("Session: {}\n", session.name));
            info.push_str(&format!("ID: {}\n", session.id));
            info.push_str(&format!("Created: {}\n", session.created_at));
            info.push_str(&format!("Status: {:?}\n", session.status));
            info.push_str(&format!("Workers: {}\n", session.workers));
            info.push_str(&format!("Log files: {}\n", session.log_files.len()));
            
            for file in &session.log_files {
                info.push_str(&format!("  - {:?}\n", file));
            }
            
            if let Some(ref config) = session.config_path {
                info.push_str(&format!("Config: {:?}\n", config));
            }
            
            Ok(info)
        }
        None => anyhow::bail!("Session '{}' not found", name),
    }
}

/// Восстановление последней активной сессии
pub fn resume_last_session() -> Result<Option<Session>> {
    let sessions = list_sessions()?;
    
    // Ищем последнюю активную сессию
    sessions.into_iter()
        .find(|s| s.status == SessionStatus::Active)
        .map_or(Ok(None), |s| Ok(Some(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_new() {
        let session = Session::new("test_session");
        
        assert_eq!(session.name, "test_session");
        assert!(!session.id.is_empty());
        assert!(!session.created_at.is_empty());
        assert!(session.log_files.is_empty());
        assert_eq!(session.status, SessionStatus::Active);
    }

    #[test]
    fn test_session_add_file() {
        let mut session = Session::new("test");
        let path = PathBuf::from("/var/log/test.log");
        
        session.add_file(path.clone());
        
        assert_eq!(session.log_files.len(), 1);
        assert_eq!(session.log_files[0], path);
    }

    #[test]
    fn test_session_set_config() {
        let mut session = Session::new("test");
        let config_path = PathBuf::from("/etc/log_analyzer/config.json");
        
        session.set_config(config_path.clone());
        
        assert_eq!(session.config_path, Some(config_path));
    }

    #[test]
    fn test_session_status_change() {
        let mut session = Session::new("test");
        assert_eq!(session.status, SessionStatus::Active);
        
        session.set_status(SessionStatus::Paused);
        assert_eq!(session.status, SessionStatus::Paused);
        
        session.set_status(SessionStatus::Completed);
        assert_eq!(session.status, SessionStatus::Completed);
    }

    #[test]
    fn test_session_serialization() {
        let mut session = Session::new("serialize_test");
        session.add_file(PathBuf::from("/test/path.log"));
        
        let json = serde_json::to_string(&session).unwrap();
        let loaded: Session = serde_json::from_str(&json).unwrap();
        
        assert_eq!(loaded.name, session.name);
        assert_eq!(loaded.log_files, session.log_files);
    }

    #[test]
    fn test_session_status_equality() {
        assert_eq!(SessionStatus::Active, SessionStatus::Active);
        assert_ne!(SessionStatus::Active, SessionStatus::Paused);
    }
}
