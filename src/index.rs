use std::{fs, thread};
use std::fs::File;
use std::io::{BufRead, BufReader, Error, Read};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bincode::deserialize;
use crossbeam_channel::{Receiver, Sender};
use druid::ExtEventSink;
use druid::im::Vector;
use fnv::FnvHashSet;
use human_bytes::human_bytes;
use jsonptr::{Pointer, ResolveMut};
use melt_rs::{get_search_index, get_search_index_with_prob};
use melt_rs::index::SearchIndex;
use num_format::{Locale, ToFormattedString};
use rocksdb::{BlockBasedOptions, Cache, DB, DBCompactionStyle, DBCompressionType, DBWithThreadMode, Options, SingleThreaded};
use serde_json::Value;

use crate::data::{AppState, Item, PointerState};
use crate::GLOBAL_STATE;

pub static GLOBAL_COUNT: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_COUNT_NEW: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_SIZE: AtomicUsize = AtomicUsize::new(0);
pub static GLOBAL_DATA_SIZE: AtomicU64 = AtomicU64::new(0);


pub fn search_thread(
    rx_search: Receiver<CommandMessage>,
    tx_search: Sender<CommandMessage>,
    sink: ExtEventSink) -> JoinHandle<i32> {
    socket_listener(tx_search, sink.clone());
    index_tread(rx_search, sink.clone())
}

fn index_tread(rx_search: Receiver<CommandMessage>, sink: ExtEventSink) -> JoinHandle<i32> {
    thread::spawn(move || {
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

        let conn: DBWithThreadMode<SingleThreaded> = DBWithThreadMode::open_cf(&opt, path, &["default"]).unwrap();
        let mut index = load_from_json();
        let prob = index.get_prob();
        sink.clone().add_idle_callback(move |data: &mut AppState| {
            data.index_prob = prob;
        });
        GLOBAL_COUNT.store(index.get_size(), Ordering::SeqCst);
        GLOBAL_SIZE.store(index.get_size_bytes(), Ordering::SeqCst);

        loop {
            match rx_search.recv() {
                Ok(cm) => {
                    match cm {
                        CommandMessage::Filter(query, neg_query, exact, time, limit, pointer_state) => {
                            if GLOBAL_STATE.lock().unwrap().query != query && GLOBAL_STATE.lock().unwrap().query_neg != neg_query {
                                continue;
                            }

                            sink.add_idle_callback(move |data: &mut AppState| {
                                data.ongoing_search = true;
                            });
                            let start = Instant::now();
                            let mut positive_keys = index.search(query.as_str(), exact);
                            let mut negative_keys = index.search(neg_query.as_str(), exact);

                            let duration_index = start.elapsed();
                            let start = Instant::now();
                            if !neg_query.is_empty() {
                                let set: FnvHashSet<usize> = positive_keys.iter().cloned().collect();
                                let set_neg: FnvHashSet<usize> = negative_keys.iter().cloned().collect();
                                negative_keys.retain(|x| set.contains(x));
                                positive_keys.retain(|x| !set_neg.contains(x));
                            }

                            positive_keys.reverse();
                            negative_keys.reverse();
                            let string = query.to_lowercase();
                            let lowercase = string.as_str();

                            let string_neg = neg_query.to_lowercase();
                            let lowercase_neg = string_neg.as_str();

                            let keys: Vec<Vec<u8>> = positive_keys.iter().map(|x| x.to_le_bytes().to_vec()).collect();
                            let keys_neg: Vec<Vec<u8>> = negative_keys.iter().map(|x| x.to_le_bytes().to_vec()).collect();
                            let index_hits = keys.len() + keys_neg.len();


                            let mut processed = 0;
                            let mut result: Vec<String> = vec![];
                            let chunk_size = 10;
                            keys.chunks(chunk_size).take_while(|_| (duration_index.as_millis() + start.elapsed().as_millis()) < time as u128).for_each(|v| {
                                if result.len() > limit { return; }
                                processed += chunk_size;
                                let map = conn.multi_get(v).iter()
                                    .map(|v_i_opt| {
                                        let vec_u8 = v_i_opt.clone().unwrap().unwrap();
                                        String::from_utf8_lossy(&vec_u8).to_string()
                                    }).filter(|string| is_match(exact, lowercase, string)).collect::<Vec<String>>();
                                result.extend(map);
                            });

                            keys_neg.chunks(chunk_size).take_while(|_| (duration_index.as_millis() + start.elapsed().as_millis()) < time as u128).for_each(|v| {
                                if result.len() > limit { return; }
                                processed += chunk_size;
                                let map = conn.multi_get(v).iter()
                                    .map(|v_i_opt| {
                                        let vec_u8 = v_i_opt.clone().unwrap().unwrap();
                                        String::from_utf8_lossy(&vec_u8).to_string()
                                    }).filter(|string| !is_match(exact, lowercase_neg, string) && is_match(exact, lowercase, string)).collect::<Vec<String>>();
                                result.extend(map);
                            });

                            if processed > index_hits {
                                processed = index_hits;
                            }
                            let duration_db = start.elapsed();
                            let res_size = result.len();
                            let query_time = format!("Index        {:?}\nIndex hits   {}\nRetrieve     {:?}\nProcessed    {}\nResults      {}", duration_index, index_hits.to_formatted_string(&Locale::en), duration_db, processed.to_formatted_string(&Locale::en), res_size.to_formatted_string(&Locale::en));

                            let mut items: Vector<_> = result.iter().take(limit).map(|m| Item::new(m.as_str())).collect();
                            sort_and_resolve(&mut items, &pointer_state);

                            sink.add_idle_callback(move |data: &mut AppState| {
                                data.query_time = query_time.clone();
                                data.items = items.clone();
                                data.ongoing_search = false;
                            });
                        }
                        CommandMessage::Quit => {
                            write_index_to_disk(&index);
                            return 0;
                        }
                        CommandMessage::InsertJson(cm) => {
                            let key = index.add(&cm);
                            conn.put(key.to_le_bytes(), &cm).unwrap();
                            GLOBAL_DATA_SIZE.fetch_add(cm.as_bytes().len() as u64, Ordering::SeqCst);
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
                                let i = index.add(String::from_utf8_lossy(&result1).to_string().as_str());
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

fn is_match(exact: bool, lowercase: &str, s: &String) -> bool {
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
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(100));
            sink1.add_idle_callback(move |data: &mut AppState| {
                if GLOBAL_COUNT_NEW.load(Ordering::SeqCst) > 0 {
                    data.count = format!("Documents    {} reindexing at {}", GLOBAL_COUNT.load(Ordering::SeqCst).to_formatted_string(&Locale::en).to_string(), GLOBAL_COUNT_NEW.load(Ordering::SeqCst).to_formatted_string(&Locale::en).to_string());
                } else {
                    data.count = format!("Documents    {}", GLOBAL_COUNT.load(Ordering::SeqCst).to_formatted_string(&Locale::en).to_string());
                }
                data.size = format!("Index size   {}", human_bytes(GLOBAL_SIZE.load(Ordering::SeqCst) as f64));
                data.indexed_data_in_bytes = GLOBAL_DATA_SIZE.load(Ordering::SeqCst);
                data.indexed_data_in_bytes_string = format!("Data size    {}", human_bytes(GLOBAL_DATA_SIZE.load(Ordering::SeqCst) as f64));
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
                    match line {
                        Ok(s) => {
                            match sender.send(CommandMessage::InsertJson(s)) {
                                Ok(_) => {}
                                Err(e) => {
                                    println!("{}", e)
                                }
                            };
                        }
                        Err(e) => { println!("{}", e) }
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
        Ok(file) => {
            deserialize(&file).unwrap_or(get_search_index())
        }
        Err(_) => {
            get_search_index()
        }
    }
}

fn resolve_pointers(items: &mut Vector<Item>, pointer_state: &Vector<PointerState>) -> bool {
    let mut empty_pointer: bool = true;
    pointer_state.iter().for_each(|ps| {
        if ps.checked {
            items.iter_mut().for_each(|item| {
                item.pointers.push(resolve_pointer(item.text.as_str(), ps.text.as_str()));
            });
            empty_pointer = false;
        }
    });
    empty_pointer.clone()
}

fn sort_and_resolve(items: &mut Vector<Item>, pointer_state: &Vector<PointerState>) {
    if !resolve_pointers(items, pointer_state) {
        items.sort_by(|left, right| {
            right.pointers.first().unwrap_or(&"".to_string())
                .cmp(left.pointers.first().unwrap_or(&"".to_string()))
        });

        items.iter_mut().for_each(|item| {
            item.view = item.pointers.join(" ")
        }
        );
    } else {
        items.iter_mut().for_each(|i| i.view = i.text.to_string())
    }
}

fn resolve_pointer(text: &str, ps: &str) -> String {
    let mut json: Value = match serde_json::from_str(text) {
        Ok(json) => json,
        Err(_) => {
            Value::Null
        }
    };
    let ptr = match Pointer::try_from(ps) {
        Ok(ptr) => ptr,
        Err(_) => {
            Pointer::root()
        }
    };

    let string = match json.resolve_mut(&ptr) {
        Ok(v) => { v }
        Err(_) => { &Value::Null }
    };
    if string.is_string() { return string.as_str().unwrap().to_string(); }
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