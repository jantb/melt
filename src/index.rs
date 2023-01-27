use std::io::{BufRead, BufReader, Error, Read};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{fs, thread};
use std::fs::File;
use std::thread::{JoinHandle, sleep};
use std::time::Duration;
use bincode::deserialize;
use crossbeam_channel::{bounded, Receiver, Sender};
use druid::ExtEventSink;
use melt_rs::get_search_index;
use melt_rs::index::SearchIndex;
use rayon::prelude::*;
use rocksdb::{DB, DBCompressionType, DBWithThreadMode, Options, SingleThreaded};
use crate::data::AppState;

pub static GLOBAL_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_SIZE: AtomicUsize = AtomicUsize::new(0);

pub fn search_thread(
    rx_search: Receiver<CommandMessage>, tx_res: Sender<ResultMessage>,
    sink: ExtEventSink) -> JoinHandle<i32> {
    let (tx_send, rx_send) = bounded(0);
    socket_listener(tx_send, sink.clone());
    index_tread(rx_search, tx_res, rx_send.clone())
}

fn default_conn() -> DBWithThreadMode<SingleThreaded> {
    let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
    let path = format!("{}/.melt.db", buf);
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    opts.set_compression_type(DBCompressionType::Zstd);
    opts.set_zstd_max_train_bytes(100 * 16384);
    opts.optimize_for_point_lookup(10);
    let db: DBWithThreadMode<SingleThreaded> = DBWithThreadMode::open_cf(&opts, path, &["default"]).unwrap();
    db
}

fn index_tread(rx_search: Receiver<CommandMessage>, tx_res: Sender<ResultMessage>, rx_send: Receiver<CommandMessage>) -> JoinHandle<i32> {
    thread::spawn(move || {
        let conn = default_conn();

        let mut index = load_from_json();
        GLOBAL_COUNT.store(index.get_size(), Ordering::SeqCst);
        GLOBAL_SIZE.store(index.get_size_bytes() / 1_000_000, Ordering::SeqCst);

        loop {
            match rx_search.try_recv() {
                Ok(cm) => {
                    match cm {
                        CommandMessage::Filter(query) => {
                            let keys: Vec<Vec<u8>> = index.search(&query).iter().map(|x| x.to_le_bytes().to_vec()).collect();

                            let result: Vec<String> = conn.multi_get(keys).par_iter()
                                .map(|result| result.as_ref().ok())
                                .map(|opt| opt.map(|vec| String::from_utf8(vec.clone().unwrap()).unwrap()).unwrap())
                                .filter(|s| s.contains(&query))
                                .collect();

                            tx_res.send(ResultMessage::Messages(result)).unwrap();
                        }
                        CommandMessage::Quit => {
                            write_index_to_disk(&index);

                            return 0;
                        }
                        CommandMessage::InsertJson(_) => {}
                        CommandMessage::Clear => {
                            index.clear();
                            GLOBAL_SIZE.store(index.get_size_bytes() / 1_000_000, Ordering::SeqCst);
                            let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
                            let path = format!("{}/.melt.db", buf);
                            let _ = DB::destroy(&Options::default(), path);
                        }
                    }
                }
                Err(_) => {}
            };

            match rx_send.recv_timeout(Duration::from_micros(10)) {
                Ok(cm) => {
                    match cm {
                        CommandMessage::InsertJson(cm) => {
                            let count = GLOBAL_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
                            index.add_message(&cm, count);
                            conn.put(count.to_le_bytes(), cm).unwrap();
                            let i = index.get_size();
                            if i % 10000 == 0 {
                                GLOBAL_SIZE.store(index.get_size_bytes() / 1_000_000, Ordering::SeqCst);
                            };
                        }

                        CommandMessage::Quit => {
                            write_index_to_disk(&index);
                            return 0;
                        }
                        _ => {}
                    }
                }
                Err(_) => {}
            }
        }
    })
}

fn write_index_to_disk(index: &SearchIndex) {
    let serialized: Vec<u8> = bincode::serialize(&index).unwrap();
    let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
    let path = format!("{}/.melt_index.dat", buf);

    fs::write(path, serialized).unwrap();
}


fn socket_listener(tx_send: Sender<CommandMessage>, sink: ExtEventSink) {
    let sink1 = sink.clone();
    thread::spawn(move || {
        loop {
            sleep(Duration::from_millis(100));
            sink1.add_idle_callback(move |data: &mut AppState| {
                data.count = format!("{} documents", GLOBAL_COUNT.load(Ordering::SeqCst).to_string());
                data.size = format!("{} mb index", GLOBAL_SIZE.load(Ordering::SeqCst).to_string());
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
    Filter(String),
    Clear,
    Quit,
    InsertJson(String),
}

pub enum ResultMessage {
    Messages(Vec<String>),
}

pub fn load_from_json() -> SearchIndex {
    let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
    let path = format!("{}/.melt_index.dat", buf);
    let file = get_file_as_byte_vec(&path);
    match file {
        Ok(file) => {
            deserialize(&file).unwrap()
        }
        Err(_) => {
            get_search_index()
        }
    }
}

fn get_file_as_byte_vec(filename: &String) -> Result<Vec<u8>, Error> {
    let mut f = File::open(&filename)?;
    let metadata = fs::metadata(&filename)?;
    let mut buffer = vec![0; metadata.len() as usize];
    f.read(&mut buffer)?;

    Ok(buffer)
}