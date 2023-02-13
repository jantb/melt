use std::fs::File;
use std::io::{BufRead, BufReader, Error, Read};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{fs, thread};

use bincode::deserialize;
use crossbeam_channel::{Receiver, Sender};
use druid::im::Vector;
use druid::{ExtEventSink, Target};
use fnv::FnvHashSet;
use human_bytes::human_bytes;
use jsonptr::{Pointer, ResolveMut};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{ListParams, LogParams};
use kube::{Api, Client};
use melt_rs::index::SearchIndex;
use melt_rs::{get_search_index, get_search_index_with_prob};
use num_format::{Locale, ToFormattedString};
use rocksdb::{
    BlockBasedOptions, Cache, DBCompactionStyle, DBCompressionType, DBWithThreadMode, Options,
    SingleThreaded, DB,
};
use serde_json::{json, Value};
use tokio::task::{yield_now, JoinHandle};
use tokio_stream::StreamExt;

use crate::data::{AppState, Item, PointerState};
use crate::delegate::SEARCH_RESULT;
use crate::GLOBAL_STATE;

pub static GLOBAL_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_COUNT_NEW: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_SIZE: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_DATA_SIZE: AtomicU64 = AtomicU64::new(0);
pub static COUNT_STACK: AtomicU64 = AtomicU64::new(0);

pub async fn search_thread(
    rx_search: Receiver<CommandMessage>,
    tx_search: Sender<CommandMessage>,
    sink: ExtEventSink,
) -> tokio::task::JoinHandle<i32> {
    socket_listener(tx_search.clone(), sink.clone());
    index_tread(rx_search, tx_search, sink.clone())
}

async fn pods(tx_search: Sender<CommandMessage>) -> Vec<JoinHandle<()>> {
    let mut handles = vec![];
    let client = match Client::try_default().await {
        Ok(c) => c,
        Err(e) => {
            println!("{}", e);
            return handles;
        }
    };
    let pods: Api<Pod> = Api::default_namespaced(client);
    let x = match pods.list(&ListParams::default()).await {
        Ok(c) => c,
        Err(e) => {
            println!("{}", e);
            return handles;
        }
    };
    for p in x.items {
        let name: String = match p.clone().metadata.name {
            None => {
                continue;
            }
            Some(s) => s,
        };
        let mut logs = match pods
            .log_stream(
                &name,
                &LogParams {
                    follow: true,
                    ..LogParams::default()
                },
            )
            .await
        {
            Ok(c) => c,
            Err(e) => {
                println!("{}", e);
                continue;
            }
        };

        let sender = tx_search.clone();
        handles.push(tokio::spawn(async move {
                while let Some(item) = match logs.try_next().await {
                    Ok(s) => {s}
                    Err(_) => {return }
                } {
                    yield_now().await;
                    let s = String::from_utf8_lossy(&item).to_string();
                    if !s.is_empty() {
                        let json = match is_valid_json(s.trim_end().trim()) {
                            true => s.trim_end().trim().to_string(),
                            false => json!({"pod": &p.clone().metadata.name.unwrap(), "log": s.trim_end().trim()})
                                .to_string(),
                        };
                        yield_now().await;
                        match sender.send(CommandMessage::InsertJson(json)) {
                            Ok(c) => c,
                            Err(e) => {
                                println!("{}", e);
                            }
                        };
                    }
                }
            }));
    }
    handles
}

fn is_valid_json(s: &str) -> bool {
    match serde_json::from_str::<Value>(s) {
        Ok(_) => true,
        Err(_) => false,
    }
}

fn index_tread(
    rx_search: Receiver<CommandMessage>,
    tx_search: Sender<CommandMessage>,
    sink: ExtEventSink,
) -> JoinHandle<i32> {
    tokio::spawn(async move {
        //   let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
        let path = ".melt.db";

        let cpu = num_cpus::get() as _;
        let mut opt = Options::default();

        opt.create_if_missing(true);
        opt.set_use_fsync(false);
        opt.set_compaction_style(DBCompactionStyle::Universal);
        opt.set_disable_auto_compactions(false);
        opt.increase_parallelism(cpu);
        opt.set_max_background_jobs(cpu / 3 + 1);
        opt.set_keep_log_file_num(16);
        opt.set_level_compaction_dynamic_level_bytes(true);

        opt.set_compression_type(DBCompressionType::Lz4);
        opt.set_bottommost_compression_type(DBCompressionType::Zstd);
        let dict_size = 32768;
        let max_train_bytes = dict_size * 128;
        opt.set_bottommost_compression_options(-14, 32767, 0, dict_size, true);
        opt.set_bottommost_zstd_max_train_bytes(max_train_bytes, true);

        opt.set_enable_blob_files(true);
        opt.set_min_blob_size(4096);
        opt.set_blob_file_size(268435456);
        opt.set_blob_compression_type(DBCompressionType::Zstd);
        opt.set_enable_blob_gc(true);
        opt.set_blob_gc_age_cutoff(0.25);
        opt.set_blob_gc_force_threshold(0.8);

        opt.set_bytes_per_sync(8388608);

        let cache = Cache::new_lru_cache(16 * 1024 * 1024).unwrap();
        let mut bopt = BlockBasedOptions::default();
        bopt.set_ribbon_filter(10.0);
        bopt.set_block_cache(&cache);
        bopt.set_block_size(6 * 1024);
        bopt.set_cache_index_and_filter_blocks(true);
        bopt.set_pin_l0_filter_and_index_blocks_in_cache(true);
        opt.set_block_based_table_factory(&bopt);
        opt.create_missing_column_families(true);

        let conn: DBWithThreadMode<SingleThreaded> =
            DBWithThreadMode::open_cf(&opt, path, &["default"]).unwrap();
        let mut index = load_from_json();

        GLOBAL_COUNT.store(index.get_size(), Ordering::SeqCst);
        GLOBAL_SIZE.store(index.get_size_bytes(), Ordering::SeqCst);
        let mut handles = vec![];
        loop {
            match rx_search.recv() {
                Ok(cm) => {
                    match cm {
                        CommandMessage::Filter(
                            query,
                            neg_query,
                            exact,
                            time,
                            limit,
                            pointer_state,
                        ) => {
                            let mut time = time as u128;
                            if GLOBAL_STATE.lock().unwrap().query != query
                                && GLOBAL_STATE.lock().unwrap().query_neg != neg_query
                            {
                                continue;
                            }

                            sink.add_idle_callback(move |data: &mut AppState| {
                                data.ongoing_search = true;
                            });
                            let start = Instant::now();
                            let mut positive_keys = index.search(query.as_str(), exact);
                            let mut negative_keys = index.search_or(neg_query.as_str());

                            let duration_index = start.elapsed();
                            let start = Instant::now();
                            if !neg_query.is_empty() {
                                let set: FnvHashSet<usize> =
                                    positive_keys.iter().cloned().collect();
                                let set_neg: FnvHashSet<usize> =
                                    negative_keys.iter().cloned().collect();
                                negative_keys.retain(|x| set.contains(x));
                                positive_keys.retain(|x| !set_neg.contains(x));
                            }

                            let string = query.to_lowercase();
                            let lowercase = string.as_str();

                            let string_neg = neg_query.to_lowercase();
                            let lowercase_neg = string_neg.as_str();

                            let index_hits = positive_keys.len() + negative_keys.len();

                            time -= duration_index.as_millis();

                            let mut result = search(
                                positive_keys,
                                limit,
                                |s: &String| is_match(exact, lowercase, s),
                                start,
                                time,
                                &conn,
                            );

                            result.extend(search(
                                negative_keys,
                                limit - result.len(),
                                |s: &String| {
                                    !is_match(false, lowercase_neg, s)
                                        && is_match(exact, lowercase, s)
                                },
                                start,
                                time,
                                &conn,
                            ));

                            let duration_db = start.elapsed();
                            let res_size = result.len();
                            let query_time = format!("Index        {:?}\nIndex hits   {}\nRetrieve     {:?}\nResults      {}", duration_index, index_hits.to_formatted_string(&Locale::en), duration_db, res_size.to_formatted_string(&Locale::en));

                            let mut items: Box<Vector<_>> = Box::new(
                                result
                                    .iter()
                                    .take(limit)
                                    .map(|m| Item::new(m.as_str()))
                                    .collect(),
                            );

                            sort_and_resolve(&mut items, &pointer_state);

                            sink.add_idle_callback(move |data: &mut AppState| {
                                data.query_time = query_time.clone();
                                data.items = *items;
                                data.ongoing_search = false;
                            });
                            sink.submit_command(SEARCH_RESULT, (), Target::Auto)
                                .unwrap();
                        }
                        CommandMessage::Pod => handles.extend(pods(tx_search.clone()).await),
                        CommandMessage::Quit => {
                            handles.iter().for_each(|h| h.abort());
                            write_index_to_disk(&index);
                            return 0;
                        }
                        CommandMessage::InsertJson(cm) => {
                            let key = index.add(&cm);
                            conn.put(key.to_le_bytes(), &cm).unwrap();
                            GLOBAL_DATA_SIZE
                                .fetch_add(cm.as_bytes().len() as u64, Ordering::SeqCst);
                            GLOBAL_COUNT.store(1 + key, Ordering::SeqCst);
                            if key % 100000 == 0 {
                                GLOBAL_SIZE.store(index.get_size_bytes(), Ordering::SeqCst);
                            };
                        }

                        CommandMessage::SetProb(prob) => {
                            let size = index.get_size();
                            index = get_search_index_with_prob(prob);
                            for x in 1..size {
                                let result1 = conn.get(x.to_le_bytes()).unwrap().unwrap();
                                let i = index
                                    .add(String::from_utf8_lossy(&result1).to_string().as_str());
                                if i % 10000 == 0 {
                                    GLOBAL_SIZE.store(index.get_size_bytes(), Ordering::SeqCst);
                                    GLOBAL_COUNT_NEW.store(i, Ordering::SeqCst);
                                }
                            }
                            GLOBAL_COUNT_NEW.store(0, Ordering::SeqCst);
                        }

                        CommandMessage::Clear => {
                            index.clear();
                            GLOBAL_SIZE.store(0, Ordering::SeqCst);
                            GLOBAL_COUNT.store(0, Ordering::SeqCst);
                            GLOBAL_DATA_SIZE.store(0, Ordering::SeqCst);
                            //    let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
                            let path = ".melt.db";
                            let _ = DB::destroy(&Options::default(), path);
                        }
                    }
                }
                Err(_) => {}
            };
        }
    })
}

fn search(
    keys: Vec<usize>,
    limit: usize,
    filter: impl Fn(&String) -> bool,
    start: Instant,
    time: u128,
    conn: &DBWithThreadMode<SingleThreaded>,
) -> Vec<String> {
    keys.iter()
        .rev()
        .take_while(|_| start.elapsed().as_millis() < time)
        .map(|x| {
            String::from_utf8_lossy(&conn.get(x.to_le_bytes().to_vec()).unwrap().unwrap())
                .to_string()
        })
        .filter(filter)
        .take(limit)
        .collect::<Vec<String>>()
}

fn is_match(exact: bool, lowercase: &str, s: &str) -> bool {
    if exact {
        s.to_lowercase().contains(lowercase)
    } else {
        lowercase.split(" ").all(|q| s.to_lowercase().contains(q))
    }
}

fn write_index_to_disk(index: &SearchIndex) {
    let serialized: Vec<u8> = bincode::serialize(&index).unwrap();
    //  let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
    let path = ".melt_index.dat";

    fs::write(path, serialized).unwrap();
}

fn socket_listener(tx_send: Sender<CommandMessage>, sink: ExtEventSink) {
    let sink1 = sink.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(100));
        sink1.add_idle_callback(move |data: &mut AppState| {
            if GLOBAL_COUNT_NEW.load(Ordering::SeqCst) > 0 {
                data.count = format!(
                    "Documents    {} reindexing at {}",
                    GLOBAL_COUNT
                        .load(Ordering::SeqCst)
                        .to_formatted_string(&Locale::en)
                        .to_string(),
                    GLOBAL_COUNT_NEW
                        .load(Ordering::SeqCst)
                        .to_formatted_string(&Locale::en)
                        .to_string()
                );
            } else {
                data.count = format!(
                    "Documents    {}",
                    GLOBAL_COUNT
                        .load(Ordering::SeqCst)
                        .to_formatted_string(&Locale::en)
                        .to_string()
                );
            }
            data.size = format!(
                "Index size   {}",
                human_bytes(GLOBAL_SIZE.load(Ordering::SeqCst) as f64)
            );
            data.indexed_data_in_bytes = GLOBAL_DATA_SIZE.load(Ordering::SeqCst);
            data.indexed_data_in_bytes_string = format!(
                "Data size    {}",
                human_bytes(GLOBAL_DATA_SIZE.load(Ordering::SeqCst) as f64)
            );
        });
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
                    match line {
                        Ok(s) => {
                            match sender.send(CommandMessage::InsertJson(s)) {
                                Ok(_) => {}
                                Err(e) => {
                                    println!("{}", e)
                                }
                            };
                        }
                        Err(e) => {
                            println!("{}", e)
                        }
                    }
                }
            });
        }
    });
}

#[derive(Clone)]
pub enum CommandMessage {
    Filter(String, String, bool, u64, usize, Vector<PointerState>),
    Clear,
    Quit,
    Pod,
    InsertJson(String),
    SetProb(f64),
}

pub enum ResultMessage {
    Messages(Vec<String>, String),
}

pub fn load_from_json() -> SearchIndex {
    //  let buf = dirs::home_dir().unwrap().into_os_string().into_string().unwrap();
    let path = ".melt_index.dat";
    let file = get_file_as_byte_vec(&path);
    match file {
        Ok(file) => deserialize(&file).unwrap_or(get_search_index()),
        Err(_) => get_search_index(),
    }
}

fn resolve_pointers(items: &mut Box<Vector<Item>>, pointer_state: &Vector<PointerState>) -> bool {
    let mut empty_pointer: bool = true;
    pointer_state.iter().for_each(|ps| {
        if ps.checked {
            items.iter_mut().for_each(|item| {
                item.pointers
                    .push(resolve_pointer(item.text.as_str(), ps.text.as_str()));
            });
            empty_pointer = false;
        }
    });
    empty_pointer.clone()
}

fn sort_and_resolve(items: &mut Box<Vector<Item>>, pointer_state: &Vector<PointerState>) {
    if !resolve_pointers(items, pointer_state) {
        items
            .iter_mut()
            .for_each(|item| item.view = item.pointers.join(" "));
        items.sort_by(|left, right| right.view.cmp(&left.view));
    } else {
        items.iter_mut().for_each(|i| i.view = i.text.to_string())
    }
}

fn resolve_pointer(text: &str, ps: &str) -> String {
    let mut json: Value = match serde_json::from_str(text) {
        Ok(json) => json,
        Err(_) => Value::Null,
    };
    let ptr = match Pointer::try_from(ps) {
        Ok(ptr) => ptr,
        Err(_) => Pointer::root(),
    };

    let string = match json.resolve_mut(&ptr) {
        Ok(v) => v,
        Err(_) => &Value::Null,
    };
    if string.is_string() {
        return string.as_str().unwrap().to_string();
    }
    if !string.is_null() {
        return string.to_string();
    }
    ps.to_string()
}

pub fn get_file_as_byte_vec(filename: &str) -> Result<Vec<u8>, Error> {
    let mut f = File::open(&filename)?;
    let metadata = fs::metadata(&filename)?;
    let mut buffer = vec![0; metadata.len() as usize];
    f.read(&mut buffer)?;

    Ok(buffer)
}
