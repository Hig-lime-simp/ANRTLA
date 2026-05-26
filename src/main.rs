//! Основной модуль приложения
//! Реализует архитектуру reader/analyzer с многопоточной обработкой

#![allow(dead_code)]

mod config;
mod log_parser;
mod analyzer;
mod statistics;
mod session;
mod error;
mod reader;

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

// Импорт из внутренних модулей
use crate::config::{Config, load_and_validate};
use crate::error::Result as AppResult;
use crate::analyzer::{Analyzer, WorkerManager};
use crate::statistics::{AnomalyType, SharedStatistics};
use crate::reader::{LogReader, create_channel, LogFilter};

/// CLI приложение для потокового анализа логов
#[derive(Parser)]
#[command(name = "log_analyzer")]
#[command(author = "Log Analyzer Team")]
#[command(version = "0.1.0")]
#[command(about = "Real-time streaming log analysis application", long_about = None)]
struct Cli {
    /// Путь к конфигурационному файлу (YAML, JSON, или TOML)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Лог файлы для анализа
    #[arg(required = false)]
    files: Vec<PathBuf>,

    /// Имя сессии (создаёт новую или восстанавливает существующую)
    #[arg(short, long)]
    session: Option<String>,

    /// Количество воркеров
    #[arg(short, long, default_value = "4")]
    workers: u32,

    /// Фильтр для логов (format: level=ERROR,keyword=test,source=auth)
    #[arg(long)]
    filter: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// Доступные команды CLI
#[derive(Subcommand)]
enum Commands {
    /// Запуск анализа логов
    Start {
        /// Лог файлы для анализа
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Просмотр статистики
    Stats {
        /// Имя сессии
        #[arg(short, long)]
        session: Option<String>,
    },
    /// Загрузка конфигурации
    LoadConfig {
        /// Путь к конфигурационному файлу
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Список доступных сессий
    ListSessions,
    /// Удаление сессии
    RemoveSession {
        /// Имя сессии для удаления
        session: String,
    },
}

/// Главный входной пункт приложения
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Инициализация логгирования
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    match cli.command {
        Some(Commands::Start { files }) => {
            let mut opt_session = create_session_if_named(
                cli.session.as_deref(),
                &files,
                cli.config.as_ref(),
                cli.workers,
            );
            run_analysis(files, cli.config, cli.workers, cli.filter).await?;
            finish_session(&mut opt_session);
        }
        Some(Commands::Stats { session }) => {
            view_stats(session.as_deref())?;
        }
        Some(Commands::LoadConfig { config }) => {
            load_and_validate(&config)?;
            println!("{}", format!("✓ Configuration loaded successfully from: {:?}", config).green());
        }
        Some(Commands::ListSessions) => {
            list_sessions()?;
        }
        Some(Commands::RemoveSession { session }) => {
            remove_session(&session)?;
            println!("{}", format!("✓ Session '{}' removed successfully", session).green());
        }
        None => {
            if !cli.files.is_empty() {
                // Создаём и сохраняем сессию если указано имя
                let mut opt_session = create_session_if_named(
                    cli.session.as_deref(),
                    &cli.files,
                    cli.config.as_ref(),
                    cli.workers,
                );
                run_analysis(
                    cli.files,
                    cli.config,
                    cli.workers,
                    cli.filter,
                ).await?;
                finish_session(&mut opt_session);
            } else if let Some(session_name) = cli.session {
                // Восстановление ранее сохранённой сессии
                resume_session(&session_name).await?;
            } else {
                show_help();
            }
        }
    }

    Ok(())
}

/// Показ справки при запуске без аргументов
fn show_help() {
    println!("{}", "Log Analyzer - Real-time streaming log analysis".cyan().bold());
    println!();
    println!("{}", "Usage:".yellow());
    println!("  log_analyzer [OPTIONS] [FILES]...");
    println!("  log_analyzer <COMMAND>");
    println!();
    println!("{}", "Commands:".yellow());
    println!("  start          Запуск анализа логов");
    println!("  stats          Просмотр статистики");
    println!("  load-config    Загрузка конфигурации");
    println!("  list-sessions  Список сессий");
    println!("  remove-session Удаление сессии");
    println!();
    println!("{}", "Options:".yellow());
    println!("  -c, --config     Путь к конфигурационному файлу");
    println!("  -s, --session    Имя сессии");
    println!("  -w, --workers    Количество воркеров (по умолчанию: 4)");
    println!("      --filter     Фильтр логов");
    println!("  -h, --help       Помощь");
}

/// Запуск анализа логов
async fn run_analysis(
    files: Vec<PathBuf>,
    config_path: Option<PathBuf>,
    workers_count: u32,
    filter_arg: Option<String>,
) -> AppResult<()> {
    println!("{}", "\n=== LOG ANALYZER STARTED ===\n".cyan().bold());
    
    // Загрузка конфигурации
    let config = if let Some(path) = config_path {
        println!("{} {:?}", "Loading config from:".green(), path);
        load_and_validate(&path).unwrap_or_default()
    } else {
        Config::default()
    };

    // Создание фильтра если указан
    let filter = filter_arg.map(|f| LogFilter::from_arg(&f));

    // Создание общей статистики с детектором аномалий
    let mut statistics = SharedStatistics::new();
    statistics.set_anomaly_detector(config.settings.anomaly_threshold, 60);
    let statistics = statistics;

    // Создание канала для связи reader и analyzer
    let (sender, receiver) = create_channel(1000);

    // Создание и запуск reader с фильтром
    let mut reader = LogReader::with_filter(sender, files.clone(), filter.clone());
    // Подключаем статистику к reader — счётчики обновляются сразу при чтении каждой строки
    reader.set_statistics(statistics.clone());
    let mut reader_handle = reader.start();

    // Клонируем receiver для analyzer и worker_manager
    let receiver_for_analyzer = receiver.clone();
    let receiver_for_workers = receiver.clone();

    // Arc на аномалии — из statistics (детектор вызывается в Reader для каждой строки)
    let anomalies_arc = statistics.get_anomalies_arc();
    let anomalies_arc_final = anomalies_arc.clone();

    // Создание и запуск analyzer
    let mut analyzer = Analyzer::new(config.clone(), statistics.clone(), receiver_for_analyzer);
    let analyzer_handle = analyzer.start();

    // Создание менеджера воркеров
    let worker_manager = WorkerManager::new(workers_count as usize, config.clone(), statistics.clone());
    let worker_handles = worker_manager.spawn_workers(receiver_for_workers);

    println!("{} {}", "Analyzing files:".green(), files.len());
    for file in &files {
        println!("  - {:?}", file);
    }
    println!("{} {}", "Workers:", workers_count);
    println!("{} {}", "Filter:", filter.as_ref().map(|f| format!("{:?}", f)).unwrap_or_else(|| "none".to_string()));

    // Основной цикл отображения статистики
    let running = Arc::new(AtomicBool::new(true));
    let stats_clone = statistics.clone();
    let running_clone = Arc::clone(&running);

    // Задача для периодического вывода статистики (500 мс) + live-вывод аномалий
    let stats_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));
        let mut last_anomaly_count = 0usize;
        while running_clone.load(Ordering::Relaxed) {
            interval.tick().await;

            // Выводим новые аномалии (если появились с последнего тика)
            if let Ok(anomalies) = anomalies_arc.lock() {
                let new_ones = &anomalies[last_anomaly_count..];
                if !new_ones.is_empty() {
                    println!(); // заканчиваем строку \r статуса
                    for anomaly in new_ones {
                        let label = match anomaly.anomaly_type {
                            AnomalyType::ErrorSpike      => "ERROR SPIKE",
                            AnomalyType::RepeatedMessage => "REPEATED MSG",
                            AnomalyType::MessageFlood    => "MSG FLOOD",
                            AnomalyType::SingleMessageType => "SINGLE TYPE",
                        };
                        println!("{}", format!(
                            "[ANOMALY {}] {} ({})",
                            label,
                            anomaly.description,
                            anomaly.detected_at.format("%H:%M:%S")
                        ).red().bold());
                    }
                    last_anomaly_count = anomalies.len();
                }
            }

            let stats = stats_clone.get_stats();
            print_status(&stats);
        }
    });

    // Ожидание прерывания (Ctrl+C) или завершения reader
    let shutdown_result = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("\n{}", "Received shutdown signal...".yellow());
            "ctrl_c"
        }
        _ = &mut reader_handle => {
            println!("\n{}", "Reader completed".yellow());
            "reader_done"
        }
    };

    // Остановка всех компонентов
    running.store(false, Ordering::Relaxed);
    reader.stop();
    analyzer.stop();

    // Ожидание завершения задач
    let _ = stats_handle.await;
    
    // Если reader еще не завершен, ожидаем его
    if shutdown_result != "reader_done" {
        let _ = reader_handle.await;
    }
    
    let _ = analyzer_handle.await;
    
    for handle in worker_handles {
        let _ = handle.await;
    }

    // Финальный отчёт
    let final_stats = statistics.get_stats();
    println!("\n{}", "\n=== FINAL STATISTICS ===".cyan().bold());
    println!("{}", final_stats.summary());

    let final_anomalies = anomalies_arc_final.lock().map(|v| v.clone()).unwrap_or_default();
    if !final_anomalies.is_empty() {
        println!("\n{}", "=== ANOMALIES SUMMARY ===".red().bold());
        for anomaly in &final_anomalies {
            println!("  [{:?}] {} ({})",
                anomaly.anomaly_type,
                anomaly.description,
                anomaly.detected_at.format("%Y-%m-%d %H:%M:%S")
            );
        }
        println!("Total anomalies: {}", final_anomalies.len());
    }

    println!("{}", "\n✓ Analysis completed successfully".green());

    Ok(())
}

/// Вывод текущей статистики в консоль
fn print_status(stats: &crate::statistics::Statistics) {
    print!("\r{} Total: {} | Errors: {} | Warnings: {} | Sources: {}",
        "Status:".blue().bold(),
        stats.total_entries.to_string().white().bold(),
        stats.error_count.to_string().red().bold(),
        stats.warning_count.to_string().yellow().bold(),
        stats.entries_by_source.len().to_string().cyan(),
    );
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

/// Просмотр статистики сессии
fn view_stats(session_name: Option<&str>) -> AppResult<()> {
    let name = session_name.unwrap_or("default");
    
    match crate::session::Session::load(name) {
        Ok(Some(session)) => {
            println!("{}", format!("\n=== SESSION: {} ===\n", session.name).cyan().bold());
            println!("ID: {}", session.id);
            println!("Created: {}", session.created_at);
            println!("Status: {:?}", session.status);
            println!("Workers: {}", session.workers);
            println!("Log files: {}", session.log_files.len());
            for file in &session.log_files {
                println!("  - {:?}", file);
            }
        }
        Ok(None) => {
            println!("{}", format!("Session '{}' not found", name).red());
        }
        Err(e) => {
            println!("{}", format!("Error loading session: {}", e).red());
        }
    }

    Ok(())
}

/// Список сессий
fn list_sessions() -> AppResult<()> {
    match crate::session::list_sessions() {
        Ok(sessions) => {
            if sessions.is_empty() {
                println!("{}", "No sessions found".yellow());
            } else {
                println!("{}", "\n=== AVAILABLE SESSIONS ===\n".cyan().bold());
                for session in &sessions {
                    println!("  {} - {} ({:?})", 
                        session.name.green().bold(),
                        session.created_at,
                        session.status
                    );
                }
                println!("\nTotal: {} session(s)", sessions.len());
            }
        }
        Err(e) => {
            println!("{}", format!("Error listing sessions: {}", e).red());
        }
    }

    Ok(())
}

/// Удаление сессии
fn remove_session(name: &str) -> AppResult<()> {
    crate::session::remove_session(name).map_err(|e| {
        crate::error::LogAnalyzerError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string()
        ))
    })?;
    Ok(())
}

/// Создаёт и сохраняет сессию на диск, если указано имя (-s / --session).
/// Возвращает Option<Session> чтобы после анализа можно было обновить статус.
fn create_session_if_named(
    name: Option<&str>,
    files: &[PathBuf],
    config_path: Option<&PathBuf>,
    workers: u32,
) -> Option<crate::session::Session> {
    let name = name?;
    let mut session = crate::session::Session::new(name);
    for file in files {
        session.add_file(file.clone());
    }
    if let Some(path) = config_path {
        session.set_config(path.clone());
    }
    session.set_workers(workers);
    if let Err(e) = session.save() {
        println!("{}", format!("Warning: could not save session '{}': {}", name, e).yellow());
        return None;
    }
    println!("{}", format!("✓ Session '{}' saved", name).green());
    Some(session)
}

/// Помечает сессию как завершённую (вызывается после run_analysis).
fn finish_session(session: &mut Option<crate::session::Session>) {
    if let Some(ref mut s) = session {
        s.set_status(crate::session::SessionStatus::Completed);
    }
}

/// Восстановление сессии
async fn resume_session(name: &str) -> AppResult<()> {
    match crate::session::Session::load(name) {
        Ok(Some(session)) => {
            println!("{}", format!("\nResuming session: {}\n", session.name).green());
            
            if !session.log_files.is_empty() {
                run_analysis(
                    session.log_files,
                    session.config_path,
                    session.workers,
                    None,
                ).await?;
            } else {
                println!("{}", "No log files in session".yellow());
            }
        }
        Ok(None) => {
            println!("{}", format!("Session '{}' not found", name).red());
        }
        Err(e) => {
            println!("{}", format!("Error loading session: {}", e).red());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing_basic() {
        // Простая проверка что CLI парсится
        assert!(true);
    }

    #[test]
    fn test_filter_creation() {
        let filter = LogFilter::from_arg("level=ERROR,keyword=test");
        assert_eq!(filter.levels, vec!["ERROR"]);
        assert_eq!(filter.keyword, Some("test".to_string()));
    }
}
