use clap::{Parser, Subcommand};
use colored::*;
use std::io::{self, BufRead};
use std::thread;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "chat-app")]
#[command(about = "Chat application with client and server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Server {
        #[arg(short, long, default_value = "8080")]
        port: u16,
    },
    Client {
        #[arg(short, long)]
        address: String,
        #[arg(short, long)]
        username: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Server { port } => {
            run_server(port).await?;
        }
        Commands::Client { address, username } => {
            run_client(address, username).await?;
        }
    }

    Ok(())
}

async fn run_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    println!("{}", format!("Server listening on port {}", port).green());

    let (tx, _rx) = broadcast::channel(100);

    loop {
        let (mut socket, addr) = listener.accept().await?;
        println!("{}", format!("New connection from {}", addr).cyan());

        let tx = tx.clone();
        let mut rx = tx.subscribe();

        tokio::spawn(async move {
            let (reader, mut writer) = socket.split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            reader.read_line(&mut line).await.unwrap();
            let username = line.trim().to_string();
            line.clear();

            println!("{}", format!("User '{}' connected from {}", username, addr).green());

            let welcome = format!("Welcome to the chat, {}!\n", username);
            if writer.write_all(welcome.as_bytes()).await.is_err() {
                return;
            }

            loop {
                tokio::select! {
                    result = reader.read_line(&mut line) => {
                        if result.unwrap_or(0) == 0 {
                            break;
                        }

                        let message = line.trim();
                        if !message.is_empty() {
                            let formatted_message = format!("{}: {}\n", username, message);
                            let _ = tx.send((formatted_message.clone(), username.clone()));
                        }
                        line.clear();
                    }
                    result = rx.recv() => {
                        let (msg, sender) = result.unwrap();
                        if sender != username {
                            if writer.write_all(msg.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            }

            println!("{}", format!("User '{}' disconnected", username).yellow());
        });
    }
}

async fn run_client(address: String, username: String) -> Result<(), Box<dyn std::error::Error>> {
    let stream = TcpStream::connect(&address).await?;
    println!("{}", format!("Connected to server at {}", address).green());

    let (reader, mut writer) = stream.into_split();

    writer.write_all(format!("{}\n", username).as_bytes()).await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    let username_clone = username.clone();
    let mut read_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    println!("{}", "Connection closed by server".red());
                    break;
                }
                Ok(_) => {
                    let message = line.trim();
                    if !message.is_empty() {
                        if message.contains(&format!("@{}", username_clone)) {
                            println!("{}", message.bright_yellow().bold());
                        } else {
                            println!("{}", message);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{}", format!("Error reading from server: {}", e).red());
                    break;
                }
            }
        }
    });

    let mut write_handle = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if writer.write_all(format!("{}\n", message).as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let tx_clone = tx.clone();
    let stdin_thread = thread::spawn(move || {
        let stdin = io::stdin();
        let stdin_lock = stdin.lock();
        let mut stdin_reader = io::BufReader::new(stdin_lock);
        let mut input_line = String::new();

        loop {
            input_line.clear();
            match stdin_reader.read_line(&mut input_line) {
                Ok(0) => break,
                Ok(_) => {
                    let message = input_line.trim();
                    if !message.is_empty() {
                        if tx_clone.send(message.to_string()).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{}", format!("Error reading input: {}", e).red());
                    break;
                }
            }
        }
    });

    tokio::select! {
        result = &mut read_handle => {
            let _ = result;
            write_handle.abort();
        }
        result = &mut write_handle => {
            let _ = result;
            read_handle.abort();
        }
    }
    
    drop(tx);
    let _ = stdin_thread.join();

    Ok(())
}

