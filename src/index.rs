use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender};
use druid::ExtEventSink;
use melt_rs::get_search_index;
use melt_rs::message::Message;
use crate::data::AppState;

pub fn search_thread(
    rx_search: Receiver<CommandMessage>, tx_res: Sender<ResultMessage>,
    sink: ExtEventSink) {
    let (tx_send, rx_send) = bounded(0);
    socket_listener(tx_send, sink);
    let mut command_senders = vec![];
    let mut result_receivers = vec![];
    for x in 0..num_cpus::get() {
        let (t1, t2) = bounded(0);
        let (t1res, t2res) = bounded(0);
        command_senders.push(t1);
        result_receivers.push(t2res);

        index_tread(t2, t1res, rx_send.clone(),x as u8);
    };
    thread::spawn(move || {
        loop {
            match rx_search.recv().unwrap() {
                CommandMessage::FilterRegex(cm) => {
                    command_senders.iter().for_each(|t| t.send(CommandMessage::FilterRegex(cm.clone())).unwrap());
                    tx_res.send(ResultMessage::Messages(result_receivers.iter()
                        .map(|r| match r.recv().unwrap()
                        { ResultMessage::Messages(m) => { m } }).flatten().collect())).unwrap();
                }
                CommandMessage::InsertJson(_) => {}
                CommandMessage::Quit => {
                    command_senders.iter().for_each(|t| t.send(CommandMessage::Quit).unwrap());
                    return;
                }
            }
        }
    });
}

fn index_tread(rx_search: Receiver<CommandMessage>, tx_res: Sender<ResultMessage>, rx_send: Receiver<CommandMessage>, thread:u8) {
    thread::spawn(move || {
        let mut index = get_search_index(thread);

        loop {
            match rx_search.try_recv() {
                Ok(cm) => {
                    match cm {
                        CommandMessage::FilterRegex(cm) => {
                            tx_res.send(ResultMessage::Messages(index.search(&cm))).unwrap();
                        }
                        CommandMessage::Quit => {return;}
                        CommandMessage::InsertJson(_) => {}
                    }
                }
                Err(_) => {}
            };

            match rx_send.recv_timeout(Duration::from_millis(100)) {
                Ok(cm) => {
                    match cm {
                        CommandMessage::InsertJson(cm) => {
                            index.add_message(&cm);
                        }

                        CommandMessage::FilterRegex(_) => {}
                        CommandMessage::Quit => {return;}
                    }
                }
                Err(_) => {}
            }
        }
    });
}

fn socket_listener(tx_send: Sender<CommandMessage>, sink: ExtEventSink) {
    let listener = TcpListener::bind("127.0.0.1:7999").unwrap();
    thread::spawn(move || {
        let mut count = 0 as usize;
        for stream in listener.incoming() {
            let sender = tx_send.clone();
            let sink = sink.clone();
            // Spawn a new thread to handle the connection
            thread::spawn(move || {
                let reader = BufReader::new(stream.unwrap());
                // Read lines from the socket
                for line in reader.lines() {
                    sender.send(CommandMessage::InsertJson(Message { json: false, value: line.unwrap() })).unwrap();
                    count += 1;
                    sink.add_idle_callback(move |data: &mut AppState| data.count = count.to_string());
                }
            });
        }
    });
}

#[derive(Clone)]
pub enum CommandMessage {
    FilterRegex(String),
    Quit,
    InsertJson(Message),
}

pub enum ResultMessage {
    Messages(Vec<Message>),
}