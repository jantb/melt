use std::collections::BTreeMap;
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
use melt_rs::get_search_index;
use melt_rs::index::SearchIndex;
use num_format::{Locale, ToFormattedString};
use rocksdb::{
    BlockBasedOptions, Cache, DBCompactionStyle, DBCompressionType, DBWithThreadMode, Options,
    SingleThreaded, DB,
};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_stream::StreamExt;
use uuid::Uuid;

use crate::data::{AppState, Item, PointerState};
use crate::delegate::{SEARCH, SEARCH_RESULT};
use crate::GLOBAL_STATE;

pub static GLOBAL_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_COUNT_NEW: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_DATA_SIZE: AtomicU64 = AtomicU64::new(0);
pub static COUNT_STACK: AtomicU64 = AtomicU64::new(0);

struct MemStore {
    lines: BTreeMap<String, String>,
    index: SearchIndex,
    conn: DBWithThreadMode<SingleThreaded>,
    bytes: usize,
}

impl MemStore {
    fn clear(&mut self) {
        self.lines.clear();
        self.index.clear();
        let _ = DB::destroy(&Options::default(), ".melt.db");
    }

    fn add(&mut self, sort_column: &str, value: &str) {
        if self.lines.len() >= 10000 {
            let x = &self.lines.pop_last().unwrap().1;
            let i = self.index.add(value.to_string().as_str());
            self.conn.put(&i.to_le_bytes(), x).unwrap();
        }
        self.lines
            .insert(sort_column.to_string(), value.to_string());
    }

    fn find(
        &self,
        query: &str,
        query_neq: &str,
        exact: bool,
        limit: usize,
        time: u128,
    ) -> Vec<String> {
        let mut result = self
            .lines
            .values()
            .into_iter()
            .filter(|s| {
                (query_neq.is_empty() || !self.is_match(false, &query_neq.to_lowercase(), s))
                    && self.is_match(exact, &query.to_lowercase(), s)
            })
            .take(limit)
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        if result.len() < limit {
            let mut positive_keys = self.index.search(query, exact);
            let mut negative_keys = self.index.search_or(query_neq);
            if !query_neq.is_empty() {
                let set: FnvHashSet<usize> = positive_keys.iter().cloned().collect();
                let set_neg: FnvHashSet<usize> = negative_keys.iter().cloned().collect();
                negative_keys.retain(|x| set.contains(x));
                positive_keys.retain(|x| !set_neg.contains(x));
            }
            let start = Instant::now();
            result.extend(self.internal_find(
                positive_keys,
                limit,
                |s: &String| self.is_match(exact, &query.to_lowercase(), s),
                start,
                time,
            ));
            result.extend(self.internal_find(
                negative_keys,
                limit - result.len(),
                |s: &String| {
                    !self.is_match(false, &query_neq.to_lowercase(), s)
                        && self.is_match(exact, &query.to_lowercase(), s)
                },
                start,
                time,
            ));
        }
        result
    }

    fn size(&self) -> usize {
        self.lines.len() + self.index.get_size()
    }

    fn internal_find(
        &self,
        keys: Vec<usize>,
        limit: usize,
        filter: impl Fn(&String) -> bool,
        start: Instant,
        time: u128,
    ) -> Vec<String> {
        keys.iter()
            .take_while(|_| start.elapsed().as_millis() < time)
            .map(|x| {
                String::from_utf8_lossy(&self.conn.get(x.to_le_bytes().to_vec()).unwrap().unwrap())
                    .to_string()
            })
            .filter(filter)
            .take(limit)
            .collect::<Vec<String>>()
    }

    fn is_match(&self, exact: bool, needle: &str, s: &str) -> bool {
        let haystack = s.to_lowercase();
        if needle.is_empty() {
            return true;
        }
        if exact {
            haystack.contains(needle)
        } else {
            needle.split(" ").all(|part| haystack.contains(part))
        }
    }
}

pub async fn search_thread(
    rx_search: Receiver<CommandMessage>,
    tx_search: Sender<CommandMessage>,
    sink: ExtEventSink,
) -> JoinHandle<i32> {
    socket_listener(tx_search.clone(), sink.clone());
    let s = sink.clone();
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(1000));
        let state = GLOBAL_STATE.lock().unwrap();
        if state.tail {
            s.submit_command(
                SEARCH,
                (
                    (state.query.to_string(), state.query_neg.to_string()),
                    false,
                ),
                Target::Auto,
            )
            .unwrap();
        }
    });
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
                sleep(Duration::from_millis(5000)).await;
            let mut buff = String::new();
                while let Some(item) = match logs.try_next().await {
                    Ok(s) => {s}
                    Err(_) => {return }
                } {

                    let s = String::from_utf8_lossy(&item).to_string();
                    buff.push_str(&s);
                    if !s.ends_with("\n") {
                        continue
                    }
                    let line = buff.clone();
                    buff = String::new();
                    if !line.is_empty() {
                        let json = match is_valid_json(line.trim_end().trim()) {
                            true => line.trim_end().trim().to_string(),
                            false => json!({"pod": &p.clone().metadata.name.unwrap(), "log": s.trim_end().trim()})
                                .to_string(),
                        };
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
        let mut mem_store = MemStore {
            lines: Default::default(),
            index: load_from_json(),
            conn,
            bytes: 0,
        };
        GLOBAL_COUNT.store(mem_store.size(), Ordering::SeqCst);

        let mut handles = vec![];
        loop {
            match rx_search.recv() {
                Ok(cm) => match cm {
                    CommandMessage::Filter(query, neg_query, exact, time, limit, pointer_state) => {
                        if GLOBAL_STATE.lock().unwrap().query != query
                            && GLOBAL_STATE.lock().unwrap().query_neg != neg_query
                        {
                            continue;
                        }

                        sink.add_idle_callback(move |data: &mut AppState| {
                            data.ongoing_search = true;
                        });

                        let result = mem_store.find(
                            query.as_str(),
                            neg_query.as_str(),
                            exact,
                            limit,
                            time as u128,
                        );

                        let res_size = result.len();
                        let query_time = format!(
                            "\nResults      {}",
                            res_size.to_formatted_string(&Locale::en)
                        );

                        let mut items: Box<Vector<_>> =
                            Box::new(result.iter().map(|m| Item::new(m.as_str())).collect());

                        resolve(&mut items, &pointer_state);

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
                        write_index_to_disk(&mem_store.index);
                        return 0;
                    }
                    CommandMessage::InsertJson(cm) => {
                        match resolve_pointer_some(&cm, &GLOBAL_STATE.lock().unwrap().sort) {
                            None => {
                                mem_store.add(&Uuid::new_v4().to_string(), &cm);
                            }
                            Some(p) => {
                                mem_store.add(&p, &cm);
                            }
                        };

                        GLOBAL_DATA_SIZE.fetch_add(cm.as_bytes().len() as u64, Ordering::SeqCst);
                        GLOBAL_COUNT.store(mem_store.size(), Ordering::SeqCst);
                    }

                    CommandMessage::Clear => {
                        mem_store.clear();
                        GLOBAL_COUNT.store(0, Ordering::SeqCst);
                        GLOBAL_DATA_SIZE.store(0, Ordering::SeqCst);
                    }
                },
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
        .take_while(|_| start.elapsed().as_millis() < time)
        .map(|x| {
            String::from_utf8_lossy(&conn.get(x.to_le_bytes().to_vec()).unwrap().unwrap())
                .to_string()
        })
        .filter(filter)
        .take(limit)
        .collect::<Vec<String>>()
}

fn write_index_to_disk(index: &SearchIndex) {
    let serialized: Vec<u8> = bincode::serialize(&index).unwrap();
    fs::write(".melt_index.dat", serialized).unwrap();
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
}

pub enum ResultMessage {
    Messages(Vec<String>, String),
}

pub fn load_from_json() -> SearchIndex {
    let file = get_file_as_byte_vec(&".melt_index.dat");
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

fn resolve(items: &mut Box<Vector<Item>>, pointer_state: &Vector<PointerState>) {
    if !resolve_pointers(items, pointer_state) {
        items
            .iter_mut()
            .for_each(|item| item.view = item.pointers.join(" "));
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
fn resolve_pointer_some(text: &str, ps: &str) -> Option<String> {
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
        return Some(string.as_str().unwrap().to_string());
    }
    if !string.is_null() {
        return Some(string.to_string());
    }
    None
}

pub fn get_file_as_byte_vec(filename: &str) -> Result<Vec<u8>, Error> {
    let mut f = File::open(&filename)?;
    let metadata = fs::metadata(&filename)?;
    let mut buffer = vec![0; metadata.len() as usize];
    f.read(&mut buffer)?;

    Ok(buffer)
}
