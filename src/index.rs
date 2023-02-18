use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Error, Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use std::{fs, io, thread};

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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_stream::StreamExt;
use uuid::Uuid;
use zstd::dict::{DecoderDictionary, EncoderDictionary};
use zstd::{Decoder, Encoder};

use crate::data::{AppState, Item, PointerState};
use crate::delegate::{SEARCH, SEARCH_RESULT};
use crate::GLOBAL_STATE;

pub static GLOBAL_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_COUNT_NEW: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_DATA_SIZE: AtomicU64 = AtomicU64::new(0);
pub static COUNT_STACK: AtomicU64 = AtomicU64::new(0);

struct MemStore {
    dict_enc: EncoderDictionary<'static>,
    dict_dec: DecoderDictionary<'static>,
    data_fd: File,
    ser: MemStoreSer,
}

#[derive(Serialize, Deserialize)]
struct MemStoreSer {
    dict: Vec<u8>,
    lines: BTreeMap<String, String>,
    index_fd: BTreeMap<usize, Entry>,
    index: SearchIndex,
    bytes: usize,
    bytes_internal: usize,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    offset: u64,
    len: usize,
}

impl MemStore {
    fn open() -> io::Result<Self> {
        let data_fd = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(".melt.data")?;
        let ser = MemStore::load();
        let store = MemStore {
            dict_enc: EncoderDictionary::copy(ser.dict.clone().as_slice(), 3),
            dict_dec: DecoderDictionary::copy(ser.dict.clone().as_slice()),
            data_fd,
            ser,
        };

        Ok(store)
    }

    pub fn put(&mut self, key: usize, value: &[u8]) -> io::Result<()> {
        let offset = self.data_fd.seek(SeekFrom::End(0))?;
        self.data_fd.write_all(value)?;
        self.ser.index_fd.insert(
            key,
            Entry {
                offset,
                len: value.len(),
            },
        );
        Ok(())
    }

    pub fn get(&self, key: &usize) -> io::Result<String> {
        let entry = self.ser.index_fd.get(key).unwrap();
        let mut file = &self.data_fd;
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut value = vec![0; entry.len];
        file.read_exact(&mut value)?;
        let string = String::from_utf8_lossy(self.decompress_with_dict(&value).unwrap().as_bytes())
            .to_string();
        return Ok(string);
    }

    fn clear(&mut self) {
        self.ser.lines.clear();
        self.ser.index.clear();
        self.data_fd.set_len(0).unwrap();
        self.ser.index_fd.clear();
        self.ser.dict.clear();
    }

    fn compress_with_dict(&self, input: &str) -> io::Result<Vec<u8>> {
        // Create an encoder with the dictionary
        let vec = Vec::new();
        let mut encoder = Encoder::with_prepared_dictionary(vec, &self.dict_enc)?;

        // Write the input to the encoder and flush it
        encoder.write_all(input.as_bytes())?;

        Ok(encoder.finish()?)
    }

    fn decompress_with_dict(&self, compressed_data: &[u8]) -> io::Result<String> {
        // Create a decoder with the provided dictionary

        let mut decoder = Decoder::with_prepared_dictionary(compressed_data, &self.dict_dec)?;

        // Read the decompressed data from the decoder into a string
        let mut decompressed_data = String::new();
        decoder.read_to_string(&mut decompressed_data)?;

        Ok(decompressed_data)
    }

    fn add(&mut self, sort_column: &str, value: &str) {
        self.ser.bytes += value.len();
        if self.ser.bytes_internal > 1024 * 1024 * 32 {
            if self.ser.dict.is_empty() {
                let vec = self
                    .ser
                    .lines
                    .values()
                    .map(|s| s.clone())
                    .collect::<Vec<String>>();
                self.ser.dict = zstd::dict::from_samples::<String>(&vec, 1024 * 1024 * 16).unwrap();
                self.dict_enc = EncoderDictionary::copy(&self.ser.dict, 3);
                self.dict_dec = DecoderDictionary::copy(&self.ser.dict);
            }

            let val = self.ser.lines.pop_last().unwrap().1;

            let key = self.ser.index.add(&val);
            self.ser.bytes_internal -= val.len();
            let compressed = self.compress_with_dict(&val).unwrap();
            self.put(key, &compressed).unwrap();
        }
        self.ser.bytes_internal += value.len();
        self.ser
            .lines
            .insert(sort_column.to_string(), value.to_string());
    }

    fn find(
        &mut self,
        query: &str,
        query_neq: &str,
        exact: bool,
        limit: usize,
        time: u128,
    ) -> Vec<String> {
        let query = query.to_lowercase();
        let mut result = self
            .ser
            .lines
            .values()
            .into_iter()
            .filter(|s| {
                (query_neq.is_empty() || !self.is_match(false, &query_neq.to_lowercase(), s))
                    && self.is_match(exact, &query, s)
            })
            .take(limit)
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        if result.len() < limit {
            let mut positive_keys = self.ser.index.search(&query, exact);
            let mut negative_keys = self.ser.index.search_or(query_neq);
            if !query_neq.is_empty() {
                let set: FnvHashSet<usize> = positive_keys.iter().cloned().collect();
                let set_neg: FnvHashSet<usize> = negative_keys.iter().cloned().collect();
                negative_keys.retain(|x| set.contains(x));
                positive_keys.retain(|x| !set_neg.contains(x));
            }
            let start = Instant::now();
            if result.len() < limit {
                result.extend(self.internal_find(
                    positive_keys,
                    limit - result.len(),
                    |s: &String| self.is_match(exact, &query, s),
                    start,
                    time,
                ));
            }
            if result.len() < limit {
                result.extend(self.internal_find(
                    negative_keys,
                    limit - result.len(),
                    |s: &String| {
                        !self.is_match(false, &query_neq.to_lowercase(), s)
                            && self.is_match(exact, &query, s)
                    },
                    start,
                    time,
                ));
            }
        }
        result
    }

    fn size(&self) -> usize {
        self.ser.lines.len() + self.ser.index.get_size()
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
            .map(|x| self.get(x).unwrap())
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

    fn write(&mut self) {
        let serialized: Vec<u8> = bincode::serialize(&self.ser).unwrap();
        fs::write(".melt.dat", serialized).unwrap();
        self.data_fd.sync_all().unwrap()
    }

    fn load() -> MemStoreSer {
        let file = get_file_as_byte_vec(&".melt.dat");
        match file {
            Ok(file) => deserialize(&file).unwrap_or(MemStoreSer {
                dict: vec![],
                lines: Default::default(),
                index_fd: Default::default(),
                index: get_search_index(),
                bytes: 0,
                bytes_internal: 0,
            }),
            Err(_) => MemStoreSer {
                dict: vec![],
                lines: Default::default(),
                index_fd: Default::default(),
                index: get_search_index(),
                bytes: 0,
                bytes_internal: 0,
            },
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
                    state.exact,
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
        let mut mem_store = MemStore::open().unwrap();
        GLOBAL_COUNT.store(mem_store.size(), Ordering::SeqCst);
        GLOBAL_DATA_SIZE.store(mem_store.ser.bytes as u64, Ordering::SeqCst);
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
                        let instant = Instant::now();
                        let result = mem_store.find(
                            query.as_str(),
                            neg_query.as_str(),
                            exact,
                            limit,
                            time as u128,
                        );

                        let query_time = format!(
                            "Query time   {:?}\nResults      {}",
                            instant.elapsed(),
                            result.len().to_formatted_string(&Locale::en)
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
                        mem_store.write();
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
                        GLOBAL_DATA_SIZE.store(mem_store.ser.bytes as u64, Ordering::SeqCst);
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
                                Err(_) => {
                                    return;
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
        Err(_) => return None,
    };
    let ptr = match Pointer::try_from(ps) {
        Ok(ptr) => ptr,
        Err(_) => return None,
    };

    let string = match json.resolve_mut(&ptr) {
        Ok(v) => v,
        Err(_) => return None,
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
