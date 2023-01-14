use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread;
use std::time::Duration;
use druid::ExtEventSink;
use melt_rs::get_search_index_with_pool;
use melt_rs::get_search_index;
use melt_rs::message::Message;
use crate::data::AppState;

pub fn search_thread(tx_send: SyncSender<CommandMessage>, rx: Receiver<CommandMessage>,
                     rx_search: Receiver<CommandMessage>, tx_res: SyncSender<ResultMessage>,
                     sink: ExtEventSink) {


    let listener = TcpListener::bind("127.0.0.1:7999").unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let sender = tx_send.clone();
            let stream = stream.unwrap();

            // Spawn a new thread to handle the connection
            thread::spawn(move || {
                let reader = BufReader::new(stream);

                // Read lines from the socket
                for line in reader.lines() {
                    let line = line.unwrap();
                    sender.send(CommandMessage::InsertJson(Message { json: false, value: line })).unwrap();
                }
            });
        }
    });

    let index = get_search_index();

    thread::spawn(move || {

        let mut index = get_search_index_with_pool(index.conn.clone());
        let mut count = 0;
        loop {
            match rx_search.try_recv() {
                Ok(cm) => {
                    match cm {
                        CommandMessage::FilterRegex(cm) => {
                            tx_res.send(ResultMessage::Messages(index.search(&cm))).unwrap();
                        }
                        _ => {}
                    }
                }
                Err(_) => {}
            };

            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(cm) => {
                    match cm {
                        CommandMessage::InsertJson(cm) => {
                            index.add_message(&cm);
                            count += 1;
                            sink.add_idle_callback(move |data: &mut AppState| data.count = count.to_string());
                        }
                        _ => {}
                    }
                }
                Err(_) => {}
            }
        }
    });
}

#[derive(Clone)]
pub enum CommandMessage {
    FilterRegex(String),
    InsertJson(Message),
}

pub enum ResultMessage {
    Messages(Vec<Message>),
}