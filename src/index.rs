use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::sync::Mutex;
use std::thread;
use std::thread::{JoinHandle, sleep};
use std::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender};
use druid::ExtEventSink;
use lazy_static::lazy_static;
use melt_rs::get_search_index;
use crate::data::AppState;

lazy_static! {
    static ref ARRAY: Mutex<Vec<usize>> = Mutex::new(vec![0;100]);
    static ref ARRAY_SIZE: Mutex<Vec<usize>> = Mutex::new(vec![0;100]);
}

pub fn search_thread(
    rx_search: Receiver<CommandMessage>, tx_res: Sender<ResultMessage>,
    sink: ExtEventSink) -> JoinHandle<i32> {
    let (tx_send, rx_send) = bounded(0);
    socket_listener(tx_send, sink.clone());
    let mut command_senders = vec![];
    let mut result_receivers = vec![];
    let mut handles = vec![];
    for x in 0..num_cpus::get() {
        let (t1, t2) = bounded(0);
        let (t1res, t2res) = bounded(0);
        command_senders.push(t1);
        result_receivers.push(t2res);

        handles.push(index_tread(t2, t1res, rx_send.clone(), x as u8))
    };
    let handle = thread::spawn(move || {
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
                    for x in handles {
                        x.join().unwrap();
                    }
                    return 0;
                }
            }
        }
    });
    handle
}

fn index_tread(rx_search: Receiver<CommandMessage>, tx_res: Sender<ResultMessage>, rx_send: Receiver<CommandMessage>, thread: u8) -> JoinHandle<i32> {
    thread::spawn(move || {
        let mut index = get_search_index(thread);
        loop {
            match rx_search.try_recv() {
                Ok(cm) => {
                    match cm {
                        CommandMessage::FilterRegex(cm) => {
                            tx_res.send(ResultMessage::Messages(index.search(&cm))).unwrap();
                        }
                        CommandMessage::Quit => {
                            index.save_to_json().unwrap();
                            return 0;
                        }
                        CommandMessage::InsertJson(_) => {}
                    }
                }
                Err(_) => {}
            };

            match rx_send.recv_timeout(Duration::from_micros(10)) {
                Ok(cm) => {
                    match cm {
                        CommandMessage::InsertJson(cm) => {
                            index.add_message(&cm);
                            let i = index.get_size();
                            ARRAY.lock().unwrap()[thread as usize] = i;
                            if i % 10000 == 0 {
                                ARRAY_SIZE.lock().unwrap()[thread as usize] = index.get_size_bytes() / 1_000_000;
                            };
                        }

                        CommandMessage::FilterRegex(_) => {}
                        CommandMessage::Quit => {
                            index.save_to_json().unwrap();
                            return 0;
                        }
                    }
                }
                Err(_) => {}
            }
        }
    })
}


fn socket_listener(tx_send: Sender<CommandMessage>, sink: ExtEventSink) {
    let sink1 = sink.clone();
    thread::spawn(move || {
        loop {
            sleep(Duration::from_millis(100));
            sink1.add_idle_callback(move |data: &mut AppState| {
                let x: usize = ARRAY.lock().unwrap().iter().sum();
                data.count = x.to_string();
                let x1: usize = ARRAY_SIZE.lock().unwrap().iter().sum();
                data.size = x1.to_string();
            });
        }
    });

    let listener = TcpListener::bind("127.0.0.1:7999").unwrap();


    thread::spawn(move || {
        for stream in listener.incoming() {
            let sender = tx_send.clone();
            // Spawn a new thread to handle the connection
            thread::spawn(move || {
                let reader = BufReader::new(stream.unwrap());
                // Read lines from the socket
                for line in reader.lines() {
                    sender.send(CommandMessage::InsertJson(line.unwrap())).unwrap_or(());
                }
            });
        }
    });
}

#[derive(Clone)]
pub enum CommandMessage {
    FilterRegex(String),
    Quit,
    InsertJson(String),
}

pub enum ResultMessage {
    Messages(Vec<String>),
}