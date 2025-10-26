#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{error::Error, rc::Rc, cell::RefCell, sync::{Arc, Mutex}};
use slint_rust_template::*;
use chrono::Local;
use slint::VecModel;

mod nfc_reader;
use nfc_reader::{NFCReader, NFCCardData};

slint::include_modules!();

// COMPLETE FIX - Add a Mutex to control NFC reader access

// COMPLETE FIX - Add a Mutex to control NFC reader access

fn main() -> Result<(), Box<dyn Error>> {
    let ui = AppWindow::new()?;
    let db = Arc::new(Mutex::new(slint_rust_template::connect_to_db()));

    let resident_ids: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let card_ids: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let log_ids: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    
    // Flag to pause automatic verification during card linking
    let verification_paused = Arc::new(Mutex::new(false));
    
    // CRITICAL: Add a global NFC reader lock to prevent concurrent access
    let nfc_reader_lock = Arc::new(Mutex::new(()));

    // Initialize NFC Reader
    let nfc_reader = Rc::new(RefCell::new(None::<NFCReader>));
    
    // List available readers on startup
    if let Ok(reader) = NFCReader::new() {
        match reader.list_readers() {
            Ok(readers) => {
                println!("Available NFC readers:");
                for (i, reader_name) in readers.iter().enumerate() {
                    println!("  {}. {}", i + 1, reader_name);
                }
                
                // Auto-select first reader if available
                if let Some(first_reader) = readers.first() {
                    let mut nfc = NFCReader::new().unwrap();
                    nfc.select_reader(first_reader).unwrap();
                    *nfc_reader.borrow_mut() = Some(nfc);
                    println!("Selected reader: {}", first_reader);
                    
                    // Start automatic card monitoring
                    let reader_name_clone = first_reader.clone();
                    let db_clone = Arc::clone(&db);
                    let ui_weak = ui.as_weak();
                    let verification_paused_clone = Arc::clone(&verification_paused);
                    let nfc_reader_lock_clone = Arc::clone(&nfc_reader_lock);
                    
                    std::thread::spawn(move || {
                        start_automatic_verification(
                            reader_name_clone, 
                            db_clone, 
                            ui_weak, 
                            verification_paused_clone,
                            nfc_reader_lock_clone
                        );
                    });
                    
                    println!("✓ Automatic card verification started!");
                    println!("  Residents can now tap their cards anytime\n");
                }
            }
            Err(e) => println!("No NFC readers found: {}", e),
        }
    }

    // Helper function to update resident list for dropdown
    fn update_resident_list(ui: &AppWindow, db: &Arc<Mutex<rusqlite::Connection>>, resident_ids: &Rc<RefCell<Vec<u32>>>) {
        let db_guard = db.lock().unwrap();
        if let Ok(row_data) = get_residents_data(&*db_guard) {
            let mut resident_strings = Vec::new();
            let mut ids = Vec::new();
            
            for resident in &row_data {
                resident_strings.push(format!("Apt {} - {} {}", 
                    resident.apt, 
                    resident.first_name, 
                    resident.last_name
                ).into());
                ids.push(resident.id);
            }
            
            *resident_ids.borrow_mut() = ids;
            let model = Rc::new(VecModel::from(resident_strings));
            ui.set_resident_list(slint::ModelRc::from(model));
        }
    }

    ui.on_add_resident({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let resident_ids = Rc::clone(&resident_ids);
        move |user_data: ResidentData|{
            let ui = ui_handle.unwrap();
            
            // Try to acquire lock with timeout
            let db_guard = match db.try_lock() {
                Ok(guard) => guard,
                Err(_) => {
                    println!("Database busy, waiting...");
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    db.lock().unwrap()
                }
            };
            
            match db_guard.execute("
                INSERT INTO resident (apt, first_name, last_name, linked) VALUES (?1, ?2, ?3, ?4)
                ", rusqlite::params![user_data.apt.to_string(), user_data.first_name.to_string(), 
                    user_data.last_name.to_string(), user_data.linked]) {
                Ok(_) => {
                    let row_data = get_residents_data(&*db_guard).unwrap();
                    let (table_model, ids) = convert_resident_data_vec(row_data);
                    *resident_ids.borrow_mut() = ids.clone();
                    ui.set_residents_data(table_model);
                    ui.set_info_alert("Resident Added".into());
                    
                    drop(db_guard);
                    update_resident_list(&ui, &db, &resident_ids);
                }
                Err(e) => {
                    println!("Failed to insert resident: {}", e);
                }
            }
        }
    });

    ui.on_remove_resident({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let resident_ids = Rc::clone(&resident_ids);
        move |resident_id: i32| {
            let ui = ui_handle.unwrap();
            
            let db_guard = db.lock().unwrap();
            
            match delete_resident(&*db_guard, resident_id as u32) {
                Ok(_) => {
                    // Refresh the residents data
                    let row_data = get_residents_data(&*db_guard).unwrap();
                    let (table_model, ids) = convert_resident_data_vec(row_data);
                    *resident_ids.borrow_mut() = ids.clone();
                    ui.set_residents_data(table_model);
                    
                    // Show success message
                    ui.set_info_alert("Resident has been removed".into());
                    
                    drop(db_guard);
                    
                    // Update resident list for dropdown
                    update_resident_list(&ui, &db, &resident_ids);
                    
                    // Auto-hide alert after 10 seconds
                    let ui_weak_clone = ui_handle.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(10));
                        if let Some(ui) = ui_weak_clone.upgrade() {
                            ui.set_info_alert("".into());
                        }
                    });
                }
                Err(e) => {
                    println!("Failed to delete resident: {}", e);
                    ui.set_info_alert(format!("Failed to delete resident: {}", e).into());
                }
            }
        } 
    });

    // Handle resident selection from dropdown
    ui.on_get_resident_at_index({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let resident_ids = Rc::clone(&resident_ids);
        move |index: i32| -> ResidentData {
            println!("🔍 Getting resident at index: {}", index);
            let ids = resident_ids.borrow();
            println!("📋 Available IDs: {:?}", *ids);
            
            if let Some(&db_id) = ids.get(index as usize) {
                println!("✓ Found DB ID: {}", db_id);
                let db = db.lock().unwrap();
                if let Ok(one_resident_info) = get_resident_info(&*db, db_id) {
                    let slint_resident = ResidentData {
                        id: one_resident_info.id as i32,
                        apt: one_resident_info.apt.clone().into(),
                        first_name: one_resident_info.first_name.clone().into(),
                        last_name: one_resident_info.last_name.clone().into(),
                        linked: one_resident_info.linked,
                    };
                    
                    println!("✓ Resident: {} {} (ID: {}, Apt: {})", 
                        one_resident_info.first_name, 
                        one_resident_info.last_name,
                        one_resident_info.id,
                        one_resident_info.apt);
                    
                    if let Some(ui) = ui_handle.upgrade() {
                        ui.set_resident_info(slint_resident.clone());
                    }
                    
                    return slint_resident;
                }
            }
            
            println!("✗ Resident not found at index {}", index);
            ResidentData {
                id: 0,
                apt: "".into(),
                first_name: "".into(),
                last_name: "".into(),
                linked: false,
            }
        }
    });

    // Complete card linking workflow with NFC lock
    ui.on_link_card_to_resident({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let verification_paused = Arc::clone(&verification_paused);
        let nfc_reader_lock = Arc::clone(&nfc_reader_lock);
        
        move |resident_id: i32, apt: slint::SharedString| -> slint::SharedString {
            println!("\n🔗 Link card called with:");
            println!("   Resident ID: {}", resident_id);
            println!("   Apartment: '{}'", apt);
            
            if resident_id == 0 {
                let error = "Error: No resident selected. Please select a resident first.";
                println!("✗ {}", error);
                return error.into();
            }
            
            // STEP 1: Pause verification and acquire NFC lock
            println!("\n⏸️  Pausing automatic verification...");
            *verification_paused.lock().unwrap() = true;
            
            println!("🔒 Acquiring exclusive NFC reader access...");
            let _nfc_lock = nfc_reader_lock.lock().unwrap();
            println!("✓ NFC reader lock acquired - verification thread blocked");
            
            // Wait longer to ensure verification thread has fully released everything
            std::thread::sleep(std::time::Duration::from_millis(1500));
            
            // Force disconnect any stale connections
            println!("🧹 Clearing stale NFC connections...");
            if let Ok(cleanup_reader) = NFCReader::new() {
                let _ = cleanup_reader.force_disconnect();
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            
            // STEP 2: Create reader for linking
            let reader_result = NFCReader::new().and_then(|mut r| {
                let readers = r.list_readers()?;
                if let Some(contactless_reader) = readers.iter()
                    .find(|name| name.contains("Contactless")) 
                {
                    r.select_reader(contactless_reader)?;
                    Ok(r)
                } else if let Some(first_reader) = readers.first() {
                    r.select_reader(first_reader)?;
                    Ok(r)
                } else {
                    Err("No readers found".into())
                }
            });
            
            let reader = match reader_result {
                Ok(r) => r,
                Err(e) => {
                    let error_msg = format!("Failed to initialize NFC reader: {}", e);
                    println!("✗ {}", error_msg);
                    drop(_nfc_lock); // Release NFC lock
                    *verification_paused.lock().unwrap() = false;
                    return error_msg.into();
                }
            };
            
            println!("\n=== Starting Card Linking Process ===");
            println!("Waiting for card... Please tap the card on the reader.");
            
            // STEP 3: Read card UID
            let uid = match reader.wait_for_card(15) {
                Ok(uid) => {
                    println!("✓ Card detected!");
                    println!("  UID: {}", uid);
                    uid
                }
                Err(e) => {
                    let error_msg = format!("Failed to read card: {}", e);
                    println!("✗ {}", error_msg);
                    drop(reader);
                    drop(_nfc_lock);
                    *verification_paused.lock().unwrap() = false;
                    return error_msg.into();
                }
            };

            // STEP 4: Create card data
            let added_date = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            let card_data = NFCCardData {
                uid: uid.clone(),
                resident_id: resident_id as u32,
                apt: apt.to_string(),
                added_date: added_date.clone(),
            };

            println!("  Resident ID: {}", resident_id);
            println!("  Apartment: {}", apt);
            println!("  Date: {}", added_date);

            // STEP 5: Generate and write hash
            let hash = card_data.generate_hash();
            println!("✓ Hash generated: {}", hash);

            println!("Writing hash to card... Keep card on reader!");
            match reader.write_hash_to_card(&hash, 4) {
                Ok(_) => {
                    println!("✓ Hash successfully written to card!");
                }
                Err(e) => {
                    let error_msg = format!("Failed to write to card: {}", e);
                    println!("✗ {}", error_msg);
                    drop(reader);
                    drop(_nfc_lock);
                    *verification_paused.lock().unwrap() = false;
                    return error_msg.into();
                }
            }

            // STEP 6: Release reader and NFC lock BEFORE database
            drop(reader);
            println!("✓ Card reader released");
            drop(_nfc_lock);
            println!("🔓 NFC reader lock released");

            std::thread::sleep(std::time::Duration::from_millis(300));

            // STEP 7: Database operations (NFC lock is now released)
            // Wait for database to be available
            println!("💾 Saving to database...");
            
            let db_guard = loop {
                match db.try_lock() {
                    Ok(guard) => break guard,
                    Err(_) => {
                        println!("  Database busy, waiting...");
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            };
            
            let db_result = db_guard.execute(
                "INSERT INTO card (resident_id, apt, added_date, hash) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![resident_id, apt.as_str(), added_date, hash],
            ).and_then(|_| {
                db_guard.execute(
                    "UPDATE resident SET linked = 1 WHERE id = ?1", 
                    [resident_id]
                )?;

                let log_action = format!(
                    "Card linked: UID={}, Hash={}, Resident ID={}, Apt={}", 
                    uid, &hash[..16], resident_id, apt
                );
                db_guard.execute(
                    "INSERT INTO log (action_type, action, date_time) VALUES (?1, ?2, ?3)",
                    rusqlite::params!["linked", log_action, added_date],
                )?;
                Ok(())
            });

            drop(db_guard);

            match db_result {
                Ok(_) => {
                    println!("✓ Card data saved to database!");
                    println!("=== Card Linking Complete ===");
                    
                    if let Some(ui) = ui_handle.upgrade() {
                        ui.set_verification_type(1);
                        let success_msg = format!("Card linked successfully!\nUID: {}\nHash: {}...", 
                            uid, &hash[..16]);
                        ui.set_verification_status(success_msg.into());
                        ui.set_info_alert("Card Linked successfully".into());

                    }
                    
                    println!("▶️  Resuming automatic verification in 2 seconds...\n");
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    *verification_paused.lock().unwrap() = false;
                    return "Success! Card linked.".into();
                }
                Err(e) => {
                    let error_msg = format!("Database error: {}", e);
                    println!("✗ {}", error_msg);
                    *verification_paused.lock().unwrap() = false;
                    return error_msg.into();
                }
            }
        }
    });

    ui.on_show_residents_data({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let resident_ids = Rc::clone(&resident_ids);
        move || {
            let ui = ui_handle.unwrap();
            let db_guard = db.lock().unwrap();
            let row_data = get_residents_data(&*db_guard).unwrap();
            let (table_model, ids) = convert_resident_data_vec(row_data);
            *resident_ids.borrow_mut() = ids.clone();
            ui.set_residents_data(table_model);
            
            drop(db_guard);
            update_resident_list(&ui, &db, &resident_ids);
        }
    });

    ui.on_show_one_log_info({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let log_ids = Rc::clone(&log_ids);
        move |index| {
            let ui = ui_handle.unwrap();
            
            let ids = log_ids.borrow();
            if let Some(&db_id) = ids.get(index as usize) {
                let db = db.lock().unwrap();
                if let Ok(one_log_info) = get_log_info(&*db, db_id) {
                    let slint_resident = LogData {
                        id: one_log_info.id as i32,
                        action_type: one_log_info.action_type.into(),
                        action: one_log_info.action.into(),
                        date_time: one_log_info.date_time.into(),
                    };
                    ui.set_log_info(slint_resident);
                }
            }
        } 
    });

    ui.on_show_one_card_info({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let card_ids = Rc::clone(&card_ids);
        move |index| {
            let ui = ui_handle.unwrap();
            
            let ids = card_ids.borrow();
            if let Some(&db_id) = ids.get(index as usize) {
                let db = db.lock().unwrap();
                if let Ok(one_card_info) = get_card_info(&*db, db_id) {
                    // Truncate hash to first 16 characters + "..."
                    let display_hash = if one_card_info.hash.len() > 16 {
                        format!("{}...", &one_card_info.hash[..16])
                    } else {
                        one_card_info.hash.clone()
                    };
                    
                    let slint_resident = CardData {
                        id: one_card_info.id as i32,
                        resident_id: one_card_info.resident_id as i32,
                        apt: one_card_info.apt.into(),
                        added_date: one_card_info.added_date.into(),
                        hash: display_hash.into(),  // Use truncated hash
                    };
                    ui.set_card_info(slint_resident);
                }
            }
        } 
    });

    ui.on_show_one_resident_info({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let resident_ids = Rc::clone(&resident_ids);
        move |index| {
            let ui = ui_handle.unwrap();
            
            let ids = resident_ids.borrow();
            if let Some(&db_id) = ids.get(index as usize) {
                let db = db.lock().unwrap();
                if let Ok(one_resident_info) = get_resident_info(&*db, db_id) {
                    let slint_resident = ResidentData {
                        id: one_resident_info.id as i32,
                        apt: one_resident_info.apt.into(),
                        first_name: one_resident_info.first_name.into(),
                        last_name: one_resident_info.last_name.into(),
                        linked: one_resident_info.linked,
                    };
                    ui.set_resident_info(slint_resident);
                }
            }
        } 
    });

    ui.on_search_residents({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let resident_ids = Rc::clone(&resident_ids);
        move |query, tab_index| {
            let ui = ui_handle.unwrap();
            
            if tab_index != 0 {
                return;
            }
            
            let db = db.lock().unwrap();
            let row_data = if query.is_empty() {
                get_residents_data(&*db).unwrap()
            } else {
                search_residents(&*db, query.as_str()).unwrap()
            };
            
            let (table_model, ids) = convert_resident_data_vec(row_data);
            *resident_ids.borrow_mut() = ids;
            ui.set_residents_data(table_model);
        }
    });

    ui.on_search_cards({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let card_ids = Rc::clone(&card_ids);  // Add this line
        move |query, tab_index| {
            let ui = ui_handle.unwrap();
            
            if tab_index != 1 {
                return;
            }
            
            let db = db.lock().unwrap();
            let row_data = if query.is_empty() {
                get_cards_data(&*db).unwrap()
            } else {
                search_cards(&*db, query.as_str()).unwrap()
            };
            
            let (table_model, ids) = convert_card_data_vec(row_data, &db);
            *card_ids.borrow_mut() = ids;  // Add this line to store IDs
            ui.set_cards_data(table_model);
        }
    });

    ui.on_show_card_data({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let card_ids = Rc::clone(&card_ids);  // Add this line
        move || {
            let ui = ui_handle.unwrap();
            let db_guard = db.lock().unwrap();
            let row_data = get_cards_data(&*db_guard).unwrap();
            let (table_model, ids) = convert_card_data_vec(row_data, &*db_guard);
            *card_ids.borrow_mut() = ids;  // Add this line to store IDs
            ui.set_cards_data(table_model);
            
            drop(db_guard);
        }
    });

    ui.on_show_log_data({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let log_ids = Rc::clone(&log_ids);  // Add this line
        move || {
            let ui = ui_handle.unwrap();
            let db_guard = db.lock().unwrap();
            let row_data = get_logs_data(&*db_guard).unwrap();
            let (table_model, ids) = convert_log_data_vec(row_data);
            *log_ids.borrow_mut() = ids;  // Add this line to store IDs
            ui.set_logs_data(table_model);
            
            drop(db_guard);
        }
    });

    ui.on_search_logs({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let log_ids = Rc::clone(&log_ids);  // Add this line
        move |query, tab_index| {
            let ui = ui_handle.unwrap();
            
            if tab_index != 2 {
                return;
            }
            
            let db = db.lock().unwrap();
            let row_data = if query.is_empty() {
                get_logs_data(&*db).unwrap()
            } else {
                search_logs(&*db, query.as_str()).unwrap()
            };
            
            let (table_model, ids) = convert_log_data_vec(row_data);
            *log_ids.borrow_mut() = ids;  // Add this line to store IDs
            ui.set_logs_data(table_model);
        }
    });

    update_resident_list(&ui, &db, &resident_ids);

    ui.invoke_show_residents_data();
    ui.run()?;

    Ok(())
}

// Updated verification function that respects the NFC reader lock
fn start_automatic_verification(
    reader_name: String, 
    db: Arc<Mutex<rusqlite::Connection>>,
    ui_weak: slint::Weak<AppWindow>,
    verification_paused: Arc<Mutex<bool>>,
    nfc_reader_lock: Arc<Mutex<()>>
) {
    println!("🔍 Card monitoring active - waiting for cards...\n");
    
    let mut last_uid: Option<String> = None;
    
    loop {
        // Check if paused
        if *verification_paused.lock().unwrap() {
            if last_uid.is_some() {
                println!("⏸️  Verification paused - waiting for resume");
                last_uid = None;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
            continue;
        }
        
        // Try to acquire NFC lock (non-blocking)
        let nfc_lock = match nfc_reader_lock.try_lock() {
            Ok(lock) => lock,
            Err(_) => {
                // NFC reader is being used by linking process
                std::thread::sleep(std::time::Duration::from_millis(300));
                continue;
            }
        };
        
        // We have the lock, create reader
        let reader = match NFCReader::new().and_then(|mut r| {
            r.select_reader(&reader_name)?;
            Ok(r)
        }) {
            Ok(r) => r,
            Err(_) => {
                drop(nfc_lock);
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
        };
        
        // Read UID
        let uid_result = reader.read_card_uid();
        
        // Drop reader immediately
        drop(reader);
        drop(nfc_lock); // Release NFC lock immediately after reading
        
        match uid_result {
            Ok(uid) => {
                if last_uid.as_ref() != Some(&uid) {
                    println!("\n📱 Card detected: {}", uid);
                    
                    // Acquire lock again for reading hash
                    if let Ok(_nfc_lock) = nfc_reader_lock.try_lock() {
                        let hash_result = NFCReader::new()
                            .and_then(|mut r| {
                                r.select_reader(&reader_name)?;
                                r.read_hash_from_card(4)
                            });
                        // Lock automatically released here
                        
                        if let Ok(card_hash) = hash_result {
                            if !card_hash.is_empty() {
                                println!("🔐 Hash from card: {}", card_hash);
                                
                                // Database lookup with minimal lock time
                                let verification_result = {
                                    match db.try_lock() {
                                        Ok(db) => {
                                            let result = db.query_row(
                                                "SELECT c.resident_id, c.apt, r.first_name, r.last_name, c.hash 
                                                 FROM card c 
                                                 JOIN resident r ON c.resident_id = r.id
                                                 WHERE c.hash = ?1",
                                                [&card_hash],
                                                |row| Ok((
                                                    row.get::<_, u32>(0)?, 
                                                    row.get::<_, String>(1)?, 
                                                    row.get::<_, String>(2)?, 
                                                    row.get::<_, String>(3)?, 
                                                    row.get::<_, String>(4)?
                                                ))
                                            );
                                            Some(result)
                                        }
                                        Err(_) => {
                                            // Database is locked, skip this verification
                                            None
                                        }
                                    }
                                }; // db lock released here

                                if let Some(result) = verification_result {
                                    match result {
                                        Ok((_, apt, first_name, last_name, stored_hash)) if stored_hash == card_hash => {
                                            let success_msg = format!(
                                                "✓ VERIFIED\n{} {}\nApartment: {}", 
                                                first_name, last_name, apt
                                            );
                                            
                                            println!("✅ {}", success_msg.replace("\n", " | "));
                                            
                                            if let Some(ui) = ui_weak.upgrade() {
                                                ui.set_verification_type(1);
                                                ui.set_verification_status(success_msg.into());
                                                ui.set_last_verified_name(format!("{} {}", first_name, last_name).into());
                                                ui.set_last_verified_apt(apt.clone().into());
                                            }
                                            
                                            

                                            // Log verification in separate short lock
                                            if let Ok(db) = db.try_lock() {
                                                let log_action = format!("Access granted: {} {} (Apt: {})", 
                                                    first_name, last_name, apt);
                                                let date_time = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
                                                let _ = db.execute(
                                                    "INSERT INTO log (action_type, action, date_time) VALUES (?1, ?2, ?3)",
                                                    rusqlite::params!["verified", log_action, date_time],
                                                );
                                            }
                                        }
                                        _ => {
                                            println!("❌ Card not registered or invalid");
                                            if let Some(ui) = ui_weak.upgrade() {
                                                ui.set_verification_type(2);
                                                ui.set_verification_status("UNKNOWN CARD".into());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    last_uid = Some(uid);
                }
            }
            Err(_) => {
                if last_uid.is_some() {
                    println!("📤 Card removed\n");
                    last_uid = None;
                    
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_verification_type(0);
                        ui.set_verification_status("".into());
                        ui.set_last_verified_name("".into());
                        ui.set_last_verified_apt("".into());
                    }
                }
            }
        }
        
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}