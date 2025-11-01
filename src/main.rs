#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{error::Error, rc::Rc, cell::RefCell, sync::{Arc, Mutex}};
use slint_rust_template::*;
use chrono::Local;
use slint::{VecModel, Model};

mod nfc_reader;
use nfc_reader::{NFCReader, NFCCardData};

slint::include_modules!();

// COMPLETE FIX - Add a Mutex to control NFC reader access

fn main() -> Result<(), Box<dyn Error>> {
    let ui = AppWindow::new()?;
    let db = Arc::new(Mutex::new(slint_rust_template::connect_to_db()));

    let resident_ids: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let card_ids: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let log_ids: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let package_ids: Rc<RefCell<Vec<u32>>> = Rc::new(RefCell::new(Vec::new()));
    let unassigned_packages: Rc<RefCell<Vec<(String, String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    
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

    ui.on_quick_scan_package({
        let ui_handle = ui.as_weak();
        let unassigned = Rc::clone(&unassigned_packages);
        
        move |barcode: slint::SharedString, comment: slint::SharedString| {
            let ui = ui_handle.unwrap();
            
            // Store temporarily with empty apartment
            unassigned.borrow_mut().push((
                barcode.to_string(),
                comment.to_string(),
                String::new()  // Empty apartment initially
            ));
            
            // Convert to Slint model for display
            let packages: Vec<PackageData> = unassigned.borrow()
                .iter()
                .enumerate()
                .map(|(idx, (bc, cmt, apt))| PackageData {
                    id: (idx + 1) as i32,
                    apt: apt.clone().into(),
                    package_number: (idx + 1).to_string().into(),
                    barcode: bc.clone().into(),
                    comment: cmt.clone().into(),
                    date_time: "".into(),
                })
                .collect();
            
            let model = Rc::new(VecModel::from(packages));
            ui.set_unassigned_packages(slint::ModelRc::from(model));
            
            println!("📦 Scanned: {} (Total: {})", barcode, unassigned.borrow().len());
        }
    });
    
    // Assign comment to single package
    ui.on_assign_comment_to_package({
        let ui_handle = ui.as_weak();
        let unassigned = Rc::clone(&unassigned_packages);
        
        move |index: i32, comment: slint::SharedString| {
            println!("\n=== ASSIGN COMMENT ===");
            println!("Index: {}", index);
            println!("Comment: '{}'", comment);
            
            if index < 0 {
                println!("ERROR: Invalid index {}", index);
                return;
            }
            
            let ui = ui_handle.unwrap();
            
            let mut packages = unassigned.borrow_mut();
            
            if index as usize >= packages.len() {
                println!("ERROR: Index {} out of bounds", index);
                return;
            }
            
            if let Some(pkg) = packages.get_mut(index as usize) {
                println!("  Before: comment='{}'", pkg.1);
                pkg.1 = comment.to_string();
                println!("  After:  comment='{}'", pkg.1);
            }
            drop(packages);
            
            // Refresh UI
            let packages: Vec<PackageData> = unassigned.borrow()
                .iter()
                .enumerate()
                .map(|(idx, (bc, cmt, apt))| PackageData {
                    id: (idx + 1) as i32,
                    apt: apt.clone().into(),
                    package_number: (idx + 1).to_string().into(),
                    barcode: bc.clone().into(),
                    comment: cmt.clone().into(),
                    date_time: "".into(),
                })
                .collect();
            
            let model = Rc::new(VecModel::from(packages));
            ui.set_unassigned_packages(slint::ModelRc::from(model));
            
            // Keep selection active
            ui.set_selected_package_index(index);
            
            println!("✅ Comment updated for #{}", index + 1);
            println!("==================\n");
        }
    });

    // Assign apartment to single package
    // Assign apartment to single package
    ui.on_assign_apartment_to_package({
        let ui_handle = ui.as_weak();
        let unassigned = Rc::clone(&unassigned_packages);
        
        move |index: i32, apt: slint::SharedString| {
            println!("\n=== RUST: ASSIGN APARTMENT CALLBACK ===");
            println!("Received - Index: {}, Apartment: '{}'", index, apt);
            
            if index < 0 {
                println!("ERROR: Invalid index");
                return;
            }
            
            if apt.is_empty() {
                println!("ERROR: Empty apartment");
                return;
            }
            
            let ui = ui_handle.unwrap();
            
            // Get current model from UI
            let packages_model = ui.get_unassigned_packages();
            
            // Update the specific package in the model
            if let Some(mut pkg) = packages_model.row_data(index as usize) {
                println!("  Found package: barcode={}, old_apt={}", pkg.barcode, pkg.apt);
                
                // Update apartment
                pkg.apt = apt.clone();
                
                // Update in the model
                packages_model.set_row_data(index as usize, pkg.clone());
                
                println!("  Updated to: apt={}", pkg.apt);
                
                // Also update backend storage
                let mut packages = unassigned.borrow_mut();
                if let Some(backend_pkg) = packages.get_mut(index as usize) {
                    backend_pkg.2 = apt.to_string();
                    println!("  Backend updated: {}", backend_pkg.2);
                }
                
                // Clear input field
                ui.set_individual_apt("".into());
                
                println!("✅ SUCCESS: Package #{} assigned to Apt {}", index + 1, apt);
            } else {
                println!("ERROR: Could not find package at index {}", index);
            }
            
            println!("=======================================\n");
        }
    });
    
    // Bulk assign apartment to all packages
    ui.on_bulk_assign_apartment({
        let ui_handle = ui.as_weak();
        let unassigned = Rc::clone(&unassigned_packages);
        
        move |apt: slint::SharedString| {
            println!("\n=== BULK ASSIGN ===");
            println!("Apartment: '{}'", apt);
            
            let ui = ui_handle.unwrap();
            
            let mut packages = unassigned.borrow_mut();
            let count = packages.len();
            
            for pkg in packages.iter_mut() {
                pkg.2 = apt.to_string();
            }
            drop(packages);
            
            // Refresh UI
            let packages: Vec<PackageData> = unassigned.borrow()
                .iter()
                .enumerate()
                .map(|(idx, (bc, cmt, apt))| PackageData {
                    id: (idx + 1) as i32,
                    apt: apt.clone().into(),
                    package_number: (idx + 1).to_string().into(),
                    barcode: bc.clone().into(),
                    comment: cmt.clone().into(),
                    date_time: "".into(),
                })
                .collect();
            
            let model = Rc::new(VecModel::from(packages));
            ui.set_unassigned_packages(slint::ModelRc::from(model));
            
            println!("✅ {} packages → Apt {}", count, apt);
            println!("==================\n");
        }
    });
    
    // Save all packages to database
    ui.on_save_assigned_packages({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let package_ids = Rc::clone(&package_ids);
        let unassigned = Rc::clone(&unassigned_packages);
        
        move || {
            let ui = ui_handle.unwrap();
            let packages_model = ui.get_unassigned_packages();
            let db_guard = db.lock().unwrap();
            
            let mut saved_count = 0;
            let mut error_count = 0;
            
            for i in 0..packages_model.row_count() {
                if let Some(pkg) = packages_model.row_data(i) {
                    // Skip if no apartment assigned
                    if pkg.apt.is_empty() {
                        error_count += 1;
                        println!("⚠️  Skipping package #{} - no apartment assigned", i + 1);
                        continue;
                    }
                    
                    let comment = if pkg.comment.is_empty() {
                        None
                    } else {
                        Some(pkg.comment.as_str())
                    };
                    
                    match add_package(
                        &*db_guard,
                        pkg.apt.as_str(),
                        &(i + 1).to_string(),
                        pkg.barcode.as_str(),
                        comment,
                    ) {
                        Ok(_) => {
                            saved_count += 1;
                            println!("✅ Saved: Package #{} → Apt {}", i + 1, pkg.apt);
                        }
                        Err(e) => {
                            error_count += 1;
                            println!("❌ Failed to save package #{}: {}", i + 1, e);
                        }
                    }
                }
            }
            
            // Refresh package list
            let row_data = get_packages_data(&*db_guard).unwrap();
            let (table_model, ids) = convert_package_data_vec(row_data);
            *package_ids.borrow_mut() = ids;
            ui.set_packages_data(table_model);
            
            // Clear temporary storage
            unassigned.borrow_mut().clear();
            
            // Show result
            if error_count == 0 {
                ui.set_info_alert(format!("✅ {} packages saved successfully!", saved_count).into());
            } else {
                ui.set_info_alert(format!("⚠️  {} saved, {} failed", saved_count, error_count).into());
            }
            
            println!("\n📊 Final: {} saved, {} errors", saved_count, error_count);
        }
    });
    
    // Clear scanned packages
    ui.on_clear_scanned_packages({
        let ui_handle = ui.as_weak();
        let unassigned = Rc::clone(&unassigned_packages);
        
        move || {
            unassigned.borrow_mut().clear();
            
            if let Some(ui) = ui_handle.upgrade() {
                let empty: Vec<PackageData> = Vec::new();
                let model = Rc::new(VecModel::from(empty));
                ui.set_unassigned_packages(slint::ModelRc::from(model));
                ui.set_scan_count(0);
            }
            
            println!("🗑️  Cleared all scanned packages");
        }
    });

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
            
            let result = match db_result {
                Ok(_) => {
                    if let Some(ui) = ui_handle.upgrade() {
                        ui.set_info_alert("Card Linked successfully".into());
                        ui.invoke_show_residents_data();
                        ui.invoke_show_card_data();
                        ui.invoke_show_log_data();
                    }
                    "Success! Card linked.".to_string()
                }
                Err(e) => {
                    let error_msg = format!("Failed to save card to database: {}", e);
                    println!("✗ {}", error_msg);
                    error_msg
                }
            };
            
            // Resume verification
            *verification_paused.lock().unwrap() = false;
            println!("▶️ Verification resumed\n");
            
            result.into()
        }
    });

    ui.on_show_residents_data({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let resident_ids = Rc::clone(&resident_ids);
        move || {
            let ui = ui_handle.unwrap();
            let db_guard = db.lock().unwrap();
            
            if let Ok(row_data) = get_residents_data(&*db_guard) {
                let (table_model, ids) = convert_resident_data_vec(row_data);
                *resident_ids.borrow_mut() = ids;
                ui.set_residents_data(table_model);
            }
            
            drop(db_guard);
            update_resident_list(&ui, &db, &resident_ids);
        }
    });

    ui.on_show_card_data({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let card_ids = Rc::clone(&card_ids);
        move || {
            let ui = ui_handle.unwrap();
            let db = db.lock().unwrap();
            if let Ok(row_data) = get_cards_data(&*db) {
                let (table_model, ids) = convert_card_data_vec(row_data, &db);
                *card_ids.borrow_mut() = ids;
                ui.set_cards_data(table_model);
            }
        }
    });

    ui.on_show_log_data({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let log_ids = Rc::clone(&log_ids);
        move || {
            let ui = ui_handle.unwrap();
            let db = db.lock().unwrap();
            if let Ok(row_data) = get_logs_data(&*db) {
                let (table_model, ids) = convert_log_data_vec(row_data);
                *log_ids.borrow_mut() = ids;
                ui.set_logs_data(table_model);
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
                        apt: one_resident_info.apt.clone().into(),
                        first_name: one_resident_info.first_name.clone().into(),
                        last_name: one_resident_info.last_name.clone().into(),
                        linked: one_resident_info.linked,
                    };
                    ui.set_resident_info(slint_resident);
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
                    let slint_card = CardData {
                        id: one_card_info.id as i32,
                        resident_id: one_card_info.resident_id as i32,
                        apt: one_card_info.apt.clone().into(),
                        added_date: one_card_info.added_date.clone().into(),
                        hash: one_card_info.hash.clone().into(),
                    };
                    ui.set_card_info(slint_card);
                }
            }
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
                    let slint_log = LogData {
                        id: one_log_info.id as i32,
                        action_type: one_log_info.action_type.clone().into(),
                        action: one_log_info.action.clone().into(),
                        date_time: one_log_info.date_time.clone().into(),
                    };
                    ui.set_log_info(slint_log);
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
        let card_ids = Rc::clone(&card_ids);
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
            *card_ids.borrow_mut() = ids;
            ui.set_cards_data(table_model);
        }
    });

    ui.on_search_logs({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let log_ids = Rc::clone(&log_ids);
        move |query, tab_index| {
            let ui = ui_handle.unwrap();
            
            if tab_index != 3 {
                return;
            }
            
            let db = db.lock().unwrap();
            let row_data = if query.is_empty() {
                get_logs_data(&*db).unwrap()
            } else {
                search_logs(&*db, query.as_str()).unwrap()
            };
            
            let (table_model, ids) = convert_log_data_vec(row_data);
            *log_ids.borrow_mut() = ids;
            ui.set_logs_data(table_model);
        }
    });

    ui.on_add_package({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let package_ids = Rc::clone(&package_ids);
        move |package_data: PackageData| {
            let ui = ui_handle.unwrap();
            
            let db_guard = db.lock().unwrap();
            
            let comment = if package_data.comment.is_empty() {
                None
            } else {
                Some(package_data.comment.as_str())
            };
            
            match add_package(
                &*db_guard,
                package_data.apt.as_str(),
                package_data.package_number.as_str(),
                package_data.barcode.as_str(),  // Add barcode here!
                comment,
            ) {
                Ok(_) => {
                    let row_data = get_packages_data(&*db_guard).unwrap();
                    let (table_model, ids) = convert_package_data_vec(row_data);
                    *package_ids.borrow_mut() = ids;
                    ui.set_packages_data(table_model);
                    ui.set_info_alert("Package Added Successfully".into());
                    
                    println!("✅ Package added for Apt {}", package_data.apt);
                }
                Err(e) => {
                    println!("Failed to add package: {}", e);
                    ui.set_info_alert(format!("Error: {}", e).into());
                }
            }
        }
    });
    
    ui.on_show_packages_data({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let package_ids = Rc::clone(&package_ids);
        move || {
            let ui = ui_handle.unwrap();
            let db_guard = db.lock().unwrap();
            
            if let Ok(row_data) = get_packages_data(&*db_guard) {
                let package_count = row_data.len();
                let (table_model, ids) = convert_package_data_vec(row_data);
                *package_ids.borrow_mut() = ids;
                ui.set_packages_data(table_model);
                ui.set_package_count(package_count as i32);
                
                println!("📦 Showing {} pending packages", package_count);
            }
        }
    });
    
    ui.on_show_one_package_info({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let package_ids = Rc::clone(&package_ids);
        move |index| {
            let ui = ui_handle.unwrap();
            
            let ids = package_ids.borrow();
            if let Some(&db_id) = ids.get(index as usize) {
                let db = db.lock().unwrap();
                if let Ok(pkg_info) = get_package_info(&*db, db_id) {
                    let slint_package = PackageData {
                        id: pkg_info.id as i32,
                        apt: pkg_info.apt.into(),
                        package_number: pkg_info.package_number.into(),
                        barcode: pkg_info.barcode.into(),  // Add barcode!
                        comment: pkg_info.comment
                            .unwrap_or_else(|| "N/A".to_string())
                            .into(),
                        date_time: pkg_info.date_time.into(),
                    };
                    ui.set_package_info(slint_package);
                }
            }
        }
    });
    
    ui.on_search_packages({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let package_ids = Rc::clone(&package_ids);
        move |query, tab_index| {
            let ui = ui_handle.unwrap();
            
            if tab_index != 2 {  // Packages tab is index 2
                return;
            }
            
            let db = db.lock().unwrap();
            let row_data = if query.is_empty() {
                get_packages_data(&*db).unwrap()
            } else {
                search_packages(&*db, query.as_str()).unwrap()
            };
            
            let (table_model, ids) = convert_package_data_vec(row_data);
            *package_ids.borrow_mut() = ids;
            ui.set_packages_data(table_model);
        }
    });
    
    // Package Collection with NFC Card Verification
    ui.on_collect_package_with_card({
        let ui_handle = ui.as_weak();
        let db = Arc::clone(&db);
        let nfc_reader_lock = Arc::clone(&nfc_reader_lock);
        
        move |package_id: i32, _apt: slint::SharedString| -> slint::SharedString {
            println!("\n📦 Package Collection Started");
            println!("  Package ID: {}", package_id);
            
            // Pause automatic verification
            *verification_paused.lock().unwrap() = true;
            
            // Acquire NFC lock
            let _nfc_lock = nfc_reader_lock.lock().unwrap();
            
            // Wait for card
            println!("🔍 Waiting for resident card...");
            
            let reader_result = NFCReader::new().and_then(|mut r| {
                let readers = r.list_readers()?;
                if let Some(first_reader) = readers.first() {
                    r.select_reader(first_reader)?;
                    Ok(r)
                } else {
                    Err("No readers found".into())
                }
            });
            
            let reader = match reader_result {
                Ok(r) => r,
                Err(e) => {
                    println!("❌ Failed to initialize NFC reader: {}", e);
                    drop(_nfc_lock);
                    *verification_paused.lock().unwrap() = false;
                    return format!("Error: {}", e).into();
                }
            };
            
            // Read card hash
            let card_hash = match reader.wait_for_card(15) {
                Ok(uid) => {
                    println!("✅ Card detected: {}", uid);
                    match reader.read_hash_from_card(4) {
                        Ok(hash) => hash,
                        Err(e) => {
                            println!("❌ Failed to read card: {}", e);
                            drop(reader);
                            drop(_nfc_lock);
                            *verification_paused.lock().unwrap() = false;
                            return "Error: Card not registered".into();
                        }
                    }
                }
                Err(e) => {
                    println!("❌ Timeout waiting for card: {}", e);
                    drop(reader);
                    drop(_nfc_lock);
                    *verification_paused.lock().unwrap() = false;
                    return "Error: No card detected".into();
                }
            };
            
            // Release NFC resources
            drop(reader);
            drop(_nfc_lock);
            
            // Process collection in database
            let db_guard = db.lock().unwrap();
            match collect_package(&*db_guard, package_id as u32, &card_hash) {
                Ok(resident_name) => {
                    println!("✅ Package collected by: {}", resident_name);
                    
                    // Refresh package list
                    if let Ok(row_data) = get_packages_data(&*db_guard) {
                        let (table_model, ids) = convert_package_data_vec(row_data);
                        drop(db_guard);
                        
                        if let Some(ui) = ui_handle.upgrade() {
                            ui.set_packages_data(table_model);
                            ui.set_info_alert(format!("Package collected by {}", resident_name).into());
                        }
                    }
                    
                    *verification_paused.lock().unwrap() = false;
                    format!("Success: {}", resident_name).into()
                }
                Err(e) => {
                    println!("❌ Collection failed: {}", e);
                    *verification_paused.lock().unwrap() = false;
                    format!("Error: {}", e).into()
                }
            }
        }
    });

    // Add callback for collecting selected packages
ui.on_collect_selected_packages({
    let ui_handle = ui.as_weak();
    let db = Arc::clone(&db);
    let package_ids = Rc::clone(&package_ids);
    
    move |selected_ids: slint::SharedString, card_hash: slint::SharedString| {
        println!("\n📦 Collecting selected packages...");
        println!("  Card hash: {}", &card_hash[..16]);
        
        // Parse comma-separated package IDs
        let ids: Vec<u32> = selected_ids
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        
        if ids.is_empty() {
            println!("  ⚠️ No packages selected");
            return;
        }
        
        println!("  Package IDs to collect: {:?}", ids);
        
        let db_guard = db.lock().unwrap();
        let mut collected_count = 0;
        let mut resident_name = String::new();
        let mut failed_packages = Vec::new();
        
        // Collect each selected package
        for pkg_id in &ids {
            match collect_package(&db_guard, *pkg_id, card_hash.as_str()) {
                Ok(name) => {
                    collected_count += 1;
                    resident_name = name;
                    println!("  ✅ Package #{} collected", pkg_id);
                }
                Err(e) => {
                    failed_packages.push(*pkg_id);
                    println!("  ❌ Failed to collect package #{}: {}", pkg_id, e);
                }
            }
        }
        
        // Refresh package data
        let row_data = get_packages_data(&db_guard).unwrap_or_default();
        let package_count = row_data.len();
        let (table_model, new_ids) = convert_package_data_vec(row_data);
        *package_ids.borrow_mut() = new_ids;
        
        drop(db_guard);
        
        // Update UI
        if let Some(ui) = ui_handle.upgrade() {
            ui.set_packages_data(table_model);
            ui.set_package_count(package_count as i32);
            ui.set_show_package_selection(false);
            ui.set_verification_type(0);
            ui.set_verification_status("".into());
            
            // Show result message
            if collected_count > 0 {
                let message = if failed_packages.is_empty() {
                    format!("✅ {} collected {} package{}", 
                        resident_name,
                        collected_count,
                        if collected_count > 1 { "s" } else { "" }
                    )
                } else {
                    format!("⚠️ {} collected {} of {} packages", 
                        resident_name,
                        collected_count,
                        ids.len()
                    )
                };
                ui.set_info_alert(message.into());
                
                // Auto-hide alert after 5 seconds
                let ui_weak = ui_handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_info_alert("".into());
                    }
                });
            } else {
                ui.set_info_alert("❌ Failed to collect packages".into());
            }
        }
        
        println!("📊 Collection complete: {} succeeded, {} failed", 
            collected_count, failed_packages.len());
    }
});

ui.on_update_selection_count({
    let ui_handle = ui.as_weak();
    move || {
        if let Some(ui) = ui_handle.upgrade() {
            let selected = ui.get_selected_packages();
            let mut count = 0;
            
            for i in 0..selected.row_count() {
                if let Some(is_selected) = selected.row_data(i) {
                    if is_selected {
                        count += 1;
                    }
                }
            }
            
            ui.set_selection_count(count);
        }
    }
});

// Helper callback: Select all packages
ui.on_select_all_packages({
    let ui_handle = ui.as_weak();
    move || {
        if let Some(ui) = ui_handle.upgrade() {
            let packages = ui.get_resident_packages_for_collection();
            let count = packages.row_count();
            
            // Create new selection array with all true
            let selection: Vec<bool> = vec![true; count];
            let model = Rc::new(VecModel::from(selection));
            ui.set_selected_packages(slint::ModelRc::from(model));
            ui.set_selection_count(count as i32);
            
            println!("✅ Selected all {} packages", count);
        }
    }
});

// Helper callback: Deselect all packages
ui.on_deselect_all_packages({
    let ui_handle = ui.as_weak();
    move || {
        if let Some(ui) = ui_handle.upgrade() {
            let packages = ui.get_resident_packages_for_collection();
            let count = packages.row_count();
            
            // Create new selection array with all false
            let selection: Vec<bool> = vec![false; count];
            let model = Rc::new(VecModel::from(selection));
            ui.set_selected_packages(slint::ModelRc::from(model));
            ui.set_selection_count(0);
            
            println!("❌ Deselected all packages");
        }
    }
});

// Helper callback: Build ID list and call collection
ui.on_collect_selected_packages_callback({
    let ui_handle = ui.as_weak();
    move || {
        if let Some(ui) = ui_handle.upgrade() {
            let packages = ui.get_resident_packages_for_collection();
            let selected = ui.get_selected_packages();
            let card_hash = ui.get_current_card_hash();
            
            // Build comma-separated ID list
            let mut ids = Vec::new();
            for i in 0..packages.row_count() {
                if let Some(pkg) = packages.row_data(i) {
                    if i < selected.row_count() {
                        if let Some(is_selected) = selected.row_data(i) {
                            if is_selected {
                                ids.push(pkg.id.to_string());
                            }
                        }
                    }
                }
            }
            
            let ids_string = ids.join(",");
            println!("📦 Calling collect with IDs: {}", ids_string);
            
            // Call the actual collection function
            ui.invoke_collect_selected_packages(ids_string.into(), card_hash);
        }
    }
});

ui.on_toggle_package_selection({
    let ui_handle = ui.as_weak();
    move |index: i32| {
        if let Some(ui) = ui_handle.upgrade() {
            let selected_model = ui.get_selected_packages();
            
            // Get current value
            if let Some(current_value) = selected_model.row_data(index as usize) {
                // Toggle it
                let new_value = !current_value;
                
                // Update the model
                selected_model.set_row_data(index as usize, new_value);
                
                // Update the count
                let mut count = 0;
                for i in 0..selected_model.row_count() {
                    if let Some(is_selected) = selected_model.row_data(i) {
                        if is_selected {
                            count += 1;
                        }
                    }
                }
                ui.set_selection_count(count);
                
                println!("📦 Package #{} toggled to: {}", index + 1, new_value);
            }
        }
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
                                        Ok((resident_id, apt, first_name, last_name, stored_hash)) if stored_hash == card_hash => {
                                            let success_msg = format!(
                                                "✓ VERIFIED\n{} {}\nApartment: {}", 
                                                first_name, last_name, apt
                                            );

                                            if let Some(ui) = ui_weak.upgrade() {
                                                if ui.get_inventory() {
                                                    // Get packages for this resident
                                                    let db_guard = db.lock().unwrap();
                                                    let packages = get_packages_for_resident(&db_guard, &apt).unwrap_or_default();
                                                    drop(db_guard);
                                                    
                                                    if packages.is_empty() {
                                                        println!("⚠️ No packages found for Apt {}", apt);
                                                        ui.set_info_alert(format!("No packages for Apt {}", apt).into());
                                                    } else {
                                                        // Convert packages to UI model
                                                        let package_data: Vec<_> = packages.iter().map(|pkg| {
                                                            PackageData {
                                                                id: pkg.id as i32,
                                                                apt: pkg.apt.clone().into(),
                                                                package_number: pkg.package_number.clone().into(),
                                                                barcode: pkg.barcode.clone().into(),
                                                                comment: pkg.comment.clone().unwrap_or_default().into(),
                                                                date_time: pkg.date_time.clone().into(),
                                                            }
                                                        }).collect();
                                                        
                                                        // Initialize selection array (all false)
                                                        let selection: Vec<bool> = vec![false; package_data.len()];
                                                        let selection_model = Rc::new(VecModel::from(selection));
                                                        
                                                        // Send data to UI
                                                        let model = Rc::new(VecModel::from(package_data));
                                                        ui.set_resident_packages_for_collection(slint::ModelRc::from(model));
                                                        ui.set_selected_packages(slint::ModelRc::from(selection_model));
                                                        ui.set_current_card_hash(card_hash.clone().into()); // ✅ Store hash!
                                                        ui.set_show_package_selection(true);
                                                        
                                                        println!("📦 Showing {} packages for selection", packages.len());
                                                    }
                                                }
                                            }
                                            
                                            println!("✅ {}", success_msg.replace("\n", " | "));
                                            
                                            if let Some(ui) = ui_weak.upgrade() {
                                                ui.set_verification_type(1);
                                                ui.set_verification_status(success_msg.into());
                                                ui.set_last_verified_name(format!("{} {}", first_name, last_name).into());
                                                ui.set_last_verified_apt(apt.clone().into());
                                            }

                                            if let Some(ui) = ui_weak.upgrade() {
                                                if ui.get_inventory() {
                                                    // Get packages for this resident
                                                    let db_guard = db.lock().unwrap();  // ✅ Use 'db', not 'db_clone'
                                                    let packages = get_packages_for_resident(&db_guard, &apt).unwrap_or_default();
                                                    drop(db_guard);
                                                    
                                                    if packages.is_empty() {
                                                        println!("⚠️ No packages found for Apt {}", apt);
                                                        ui.set_info_alert(format!("No packages for Apt {}", apt).into());
                                                    } else {
                                                        // Convert packages to UI model
                                                        let package_data: Vec<_> = packages.iter().map(|pkg| {
                                                            PackageData {
                                                                id: pkg.id as i32,
                                                                apt: pkg.apt.clone().into(),
                                                                package_number: pkg.package_number.clone().into(),
                                                                barcode: pkg.barcode.clone().into(),
                                                                comment: pkg.comment.clone().unwrap_or_default().into(),
                                                                date_time: pkg.date_time.clone().into(),
                                                            }
                                                        }).collect();
                                                        
                                                        // Initialize selection array (all false)
                                                        let selection: Vec<bool> = vec![false; package_data.len()];
                                                        let selection_model = Rc::new(VecModel::from(selection));
                                                        
                                                        // Send data to UI
                                                        let model = Rc::new(VecModel::from(package_data));
                                                        ui.set_resident_packages_for_collection(slint::ModelRc::from(model));
                                                        ui.set_selected_packages(slint::ModelRc::from(selection_model));
                                                        ui.set_current_card_hash(card_hash.clone().into());
                                                        ui.set_show_package_selection(true);
                                                        
                                                        println!("📦 Showing {} packages for selection", packages.len());
                                                    }
                                                }
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