use rusqlite::{Connection, Error};
use std::{rc::Rc};
use slint::{VecModel, StandardListViewItem, ModelRc};

pub struct ResidentData{
    pub id: u32,
    pub apt: String,
    pub first_name: String,
    pub last_name: String,
    pub linked: bool,
}

pub struct CardData{
    pub id: u32,
    pub resident_id: u32,
    pub apt: String,
    pub added_date: String,
    pub hash: String,
}

pub struct LogData{
    pub id: u32,
    pub action_type: String,
    pub action: String,
    pub date_time: String,
}

pub struct PackageData {
    pub id: u32,
    pub apt: String,
    pub package_number: String,
    pub tracking_number: Option<String>,
    pub date_time: String,
    pub status: String, // "pending", "collected", "returned"
}

pub fn connect_to_db()->Connection{
    let db = Connection::open("package_room.db").expect("Cant connect to database");
    // Enable foreign keys
    db.execute("PRAGMA foreign_keys = ON", []).expect("Failed to enable foreign keys");
    create_tables(&db);
    db
}

fn create_tables(db: &Connection) {
    db.execute_batch("
        CREATE TABLE IF NOT EXISTS resident (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            apt TEXT NOT NULL,
            first_name TEXT NOT NULL,
            last_name TEXT NOT NULL,
            linked BOOLEAN DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS card (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            resident_id INTEGER NOT NULL,
            apt TEXT NOT NULL,
            added_date TEXT NOT NULL,
            hash TEXT NOT NULL UNIQUE,
            FOREIGN KEY (resident_id) REFERENCES resident(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action_type TEXT NOT NULL,
            action TEXT NOT NULL,
            date_time TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS package (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            apt TEXT NOT NULL,
            package_number TEXT NOT NULL,
            tracking_number TEXT,
            date_time TEXT NOT NULL,
            status TEXT DEFAULT 'pending',
            collection_time TEXT,
            collected_by INTEGER,
            FOREIGN KEY (collected_by) REFERENCES resident(id)
        );

        CREATE INDEX IF NOT EXISTS idx_card_resident ON card(resident_id);
        CREATE INDEX IF NOT EXISTS idx_log_date ON log(date_time);
        CREATE INDEX IF NOT EXISTS idx_package_apt ON package(apt);
        CREATE INDEX IF NOT EXISTS idx_package_status ON package(status);
    ").expect("Failed to create tables");
}

//Resident functions
pub fn get_residents_data(db: &Connection) -> Result<Vec<ResidentData>, Error> {
    let mut query = db.prepare("SELECT * FROM resident")?;

    let query_map = query.query_map([], |row| {
        Ok(ResidentData {
            id: row.get(0)?,
            apt: row.get(1)?,
            first_name: row.get(2)?,
            last_name: row.get(3)?,
            linked: row.get(4)?,
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}

pub fn convert_resident_data_vec(row_data: Vec<ResidentData>)-> (ModelRc<ModelRc<StandardListViewItem>>, Vec<u32>){
    let mut ids = Vec::new();

    let rows: Vec<ModelRc<StandardListViewItem>> = row_data.into_iter().map(|resident| {
        ids.push(resident.id);

        let inner_vec = vec![
            StandardListViewItem::from(Into::<slint::SharedString>::into(resident.id.to_string())),
            StandardListViewItem::from(Into::<slint::SharedString>::into(resident.apt)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(resident.first_name)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(resident.last_name)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(resident.linked.to_string())),
        ];
        let inner_model = Rc::new(VecModel::from(inner_vec));
        ModelRc::new(inner_model)
    }).collect();
    // Внешняя модель таблицы
    let outer_model = Rc::new(VecModel::from(rows));
    let table_model: ModelRc<ModelRc<StandardListViewItem>> = ModelRc::new(outer_model);
    (table_model, ids)
}

pub fn get_resident_info(db: &Connection, index: u32) -> Result<ResidentData, Error> {
    let resident = db.query_row(
        "SELECT id, apt, first_name, last_name, linked FROM resident WHERE id = ?1",
        [index],
        |row| {
            Ok(ResidentData {
                id: row.get(0)?,
                apt: row.get(1)?,
                first_name: row.get(2)?,
                last_name: row.get(3)?,
                linked: row.get(4)?,
            })
        },
    )?;
    Ok(resident)
}

pub fn get_card_info(db: &Connection, index: u32) -> Result<CardData, Error> {
    let card = db.query_row(
        "SELECT id, resident_id, apt, added_date, hash FROM card WHERE id = ?1",
        [index],
        |row| {
            Ok(CardData {
                id: row.get(0)?,
                resident_id: row.get(1)?,
                apt: row.get(2)?,
                added_date: row.get(3)?,
                hash: row.get(4)?,
            })
        },
    )?;
    Ok(card)
}

pub fn get_log_info(db: &Connection, index: u32) -> Result<LogData, Error> {
    let log = db.query_row(
        "SELECT id, action_type, action, date_time FROM log WHERE id = ?1",
        [index],
        |row| {
            Ok(LogData {
                id: row.get(0)?,
                action_type: row.get(1)?,
                action: row.get(2)?,
                date_time: row.get(3)?,
            })
        },
    )?;
    Ok(log)
}

pub fn delete_resident(db: &Connection, id: u32) -> Result<(), Error> {
    // Log the deletion
    let resident = get_resident_info(db, id)?;
    let log_action = format!("Resident {} {} (ID: {}, Apt: {}) was removed", 
        resident.first_name, resident.last_name, id, resident.apt);
    add_log(db, "remove", &log_action)?;

    // Delete resident (cards will be deleted automatically due to CASCADE)
    db.execute("DELETE FROM resident WHERE id = ?1", [id])?;
    Ok(())
}

pub fn search_residents(db: &Connection, search_query: &str) -> Result<Vec<ResidentData>, Error> {

    let query = format!("%{}%", search_query.to_lowercase());
    let mut stmt = db.prepare(
        "SELECT id, apt, first_name, last_name, linked FROM resident 
         WHERE LOWER(apt) LIKE ?1 
         OR LOWER(first_name) LIKE ?1 
         OR LOWER(last_name) LIKE ?1"
    )?;

    let query_map = stmt.query_map([&query], |row| {
        Ok(ResidentData {
            id: row.get(0)?,
            apt: row.get(1)?,
            first_name: row.get(2)?,
            last_name: row.get(3)?,
            linked: row.get(4)?,
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}

// Card functions
pub fn get_cards_data(db: &Connection) -> Result<Vec<CardData>, Error> {
    let mut query = db.prepare("SELECT * FROM card")?;

    let query_map = query.query_map([], |row| {
        Ok(CardData {
            id: row.get(0)?,
            resident_id: row.get(1)?,
            apt: row.get(2)?,
            added_date: row.get(3)?,
            hash: row.get(4)?,
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}

pub fn convert_card_data_vec(row_data: Vec<CardData>, db: &Connection) -> (ModelRc<ModelRc<StandardListViewItem>>, Vec<u32>) {
    let mut ids = Vec::new();

    let rows: Vec<ModelRc<StandardListViewItem>> = row_data.into_iter().map(|card| {
        ids.push(card.id);

        // Get resident info for display
        let resident_name = if let Ok(resident) = get_resident_info(db, card.resident_id) {
            format!("{} {}", resident.first_name, resident.last_name)
        } else {
            "Unknown".to_string()
        };

        let inner_vec = vec![
            StandardListViewItem::from(Into::<slint::SharedString>::into(card.id.to_string())),
            StandardListViewItem::from(Into::<slint::SharedString>::into(card.apt)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(resident_name)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(card.added_date)),
        ];
        let inner_model = Rc::new(VecModel::from(inner_vec));
        ModelRc::new(inner_model)
    }).collect();
    
    let outer_model = Rc::new(VecModel::from(rows));
    let table_model: ModelRc<ModelRc<StandardListViewItem>> = ModelRc::new(outer_model);
    (table_model, ids)
}

pub fn add_card(db: &Connection, resident_id: u32, apt: &str, hash: &str) -> Result<(), Error> {
    use chrono::Local;
    let added_date = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    db.execute(
        "INSERT INTO card (resident_id, apt, added_date) VALUES (?1, ?2, ?3)",
        rusqlite::params![resident_id, apt, added_date],
    )?;

    // Log the action
    let log_action = format!("Card {} was linked to resident ID: {} (Apt: {})", hash, resident_id, apt);
    add_log(db, "linked", &log_action)?;

    // Update resident linked status
    db.execute("UPDATE resident SET linked = 1 WHERE id = ?1", [resident_id])?;

    Ok(())
}

pub fn search_cards(db: &Connection, search_query: &str) -> Result<Vec<CardData>, Error> {
    let query = format!("%{}%", search_query.to_lowercase());
    let mut stmt = db.prepare(
        "SELECT c.id, c.resident_id, c.apt, c.added_date, c.hash 
         FROM card c
         JOIN resident r ON c.resident_id = r.id
         WHERE LOWER(c.apt) LIKE ?1 
         OR LOWER(r.first_name) LIKE ?1 
         OR LOWER(r.last_name) LIKE ?1"
    )?;

    let query_map = stmt.query_map([&query], |row| {
        Ok(CardData {
            id: row.get(0)?,
            resident_id: row.get(1)?,
            apt: row.get(2)?,
            added_date: row.get(3)?,
            hash: row.get(4)?,  // Now this column exists!
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}

// Log functions
pub fn get_logs_data(db: &Connection) -> Result<Vec<LogData>, Error> {
    let mut query = db.prepare("SELECT * FROM log ORDER BY date_time DESC")?;

    let query_map = query.query_map([], |row| {
        Ok(LogData {
            id: row.get(0)?,
            action_type: row.get(1)?,
            action: row.get(2)?,
            date_time: row.get(3)?,
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}

pub fn convert_log_data_vec(row_data: Vec<LogData>) -> (ModelRc<ModelRc<StandardListViewItem>>, Vec<u32>) {
    let mut ids = Vec::new();

    let rows: Vec<ModelRc<StandardListViewItem>> = row_data.into_iter().map(|log| {
        ids.push(log.id);

        let inner_vec = vec![
            StandardListViewItem::from(Into::<slint::SharedString>::into(log.id.to_string())),
            StandardListViewItem::from(Into::<slint::SharedString>::into(log.action_type)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(log.action)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(log.date_time)),
        ];
        let inner_model = Rc::new(VecModel::from(inner_vec));
        ModelRc::new(inner_model)
    }).collect();
    
    let outer_model = Rc::new(VecModel::from(rows));
    let table_model: ModelRc<ModelRc<StandardListViewItem>> = ModelRc::new(outer_model);
    (table_model, ids)
}

pub fn add_log(db: &Connection, action_type: &str, action: &str) -> Result<(), Error> {
    use chrono::Local;
    let date_time = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    db.execute(
        "INSERT INTO log (action_type, action, date_time) VALUES (?1, ?2, ?3)",
        rusqlite::params![action_type, action, date_time],
    )?;

    Ok(())
}

pub fn search_logs(db: &Connection, search_query: &str) -> Result<Vec<LogData>, Error> {
    let query = format!("%{}%", search_query.to_lowercase());
    let mut stmt = db.prepare(
        "SELECT id, action_type, action, date_time FROM log 
         WHERE LOWER(action_type) LIKE ?1 
         OR LOWER(action) LIKE ?1
         OR LOWER(date_time) LIKE ?1
         ORDER BY date_time DESC"
    )?;

    let query_map = stmt.query_map([&query], |row| {
        Ok(LogData {
            id: row.get(0)?,
            action_type: row.get(1)?,
            action: row.get(2)?,
            date_time: row.get(3)?,
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}

// Package Functions
pub fn add_package(
    db: &Connection, 
    apt: &str, 
    package_number: &str,
    tracking_number: Option<&str>
) -> Result<u32, Error> {
    use chrono::Local;
    let date_time = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    
    db.execute(
        "INSERT INTO package (apt, package_number, tracking_number, date_time, status) 
         VALUES (?1, ?2, ?3, ?4, 'pending')",
        rusqlite::params![apt, package_number, tracking_number, date_time],
    )?;
    
    let package_id = db.last_insert_rowid() as u32;
    
    // Log the action
    let log_action = format!(
        "Package received for Apt {}: {} (Tracking: {})", 
        apt, 
        package_number,
        tracking_number.unwrap_or("N/A")
    );
    add_log(db, "package_in", &log_action)?;
    
    Ok(package_id)
}

pub fn get_packages_data(db: &Connection) -> Result<Vec<PackageData>, Error> {
    let mut query = db.prepare(
        "SELECT id, apt, package_number, tracking_number, date_time, status 
         FROM package 
         WHERE status = 'pending'
         ORDER BY date_time DESC"
    )?;

    let query_map = query.query_map([], |row| {
        Ok(PackageData {
            id: row.get(0)?,
            apt: row.get(1)?,
            package_number: row.get(2)?,
            tracking_number: row.get(3)?,
            date_time: row.get(4)?,
            status: row.get(5)?,
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}

pub fn convert_package_data_vec(
    row_data: Vec<PackageData>
) -> (ModelRc<ModelRc<StandardListViewItem>>, Vec<u32>) {
    let mut ids = Vec::new();

    let rows: Vec<ModelRc<StandardListViewItem>> = row_data.into_iter().map(|package| {
        ids.push(package.id);

        let tracking = package.tracking_number.unwrap_or_else(|| "N/A".to_string());
        
        let inner_vec = vec![
            StandardListViewItem::from(Into::<slint::SharedString>::into(package.id.to_string())),
            StandardListViewItem::from(Into::<slint::SharedString>::into(package.apt)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(package.package_number)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(tracking)),
            StandardListViewItem::from(Into::<slint::SharedString>::into(package.date_time)),
        ];
        let inner_model = Rc::new(VecModel::from(inner_vec));
        ModelRc::new(inner_model)
    }).collect();
    
    let outer_model = Rc::new(VecModel::from(rows));
    let table_model: ModelRc<ModelRc<StandardListViewItem>> = ModelRc::new(outer_model);
    (table_model, ids)
}

pub fn get_package_info(db: &Connection, index: u32) -> Result<PackageData, Error> {
    let package = db.query_row(
        "SELECT id, apt, package_number, tracking_number, date_time, status 
         FROM package WHERE id = ?1",
        [index],
        |row| {
            Ok(PackageData {
                id: row.get(0)?,
                apt: row.get(1)?,
                package_number: row.get(2)?,
                tracking_number: row.get(3)?,
                date_time: row.get(4)?,
                status: row.get(5)?,
            })
        },
    )?;
    Ok(package)
}

pub fn collect_package(db: &Connection, package_id: u32, card_hash: &str) -> Result<String, Error> {
    use chrono::Local;
    
    // Verify card and get resident info
    let resident = db.query_row(
        "SELECT r.id, r.apt, r.first_name, r.last_name 
         FROM card c 
         JOIN resident r ON c.resident_id = r.id
         WHERE c.hash = ?1",
        [card_hash],
        |row| Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    )?;
    
    // Get package info
    let package = get_package_info(db, package_id)?;
    
    // Verify apartment matches
    if resident.1 != package.apt {
        return Err(Error::SqliteFailure(
            rusqlite::ffi::Error::new(1),
            Some(format!("Package is for Apt {}, but card belongs to Apt {}", package.apt, resident.1))
        ));
    }
    
    // Mark as collected
    let collection_time = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    db.execute(
        "UPDATE package SET status = 'collected', collection_time = ?1 WHERE id = ?2",
        rusqlite::params![collection_time, package_id],
    )?;
    
    // Log collection
    let log_action = format!(
        "Package collected: {} {} (Apt {}) picked up package #{}", 
        resident.2, resident.3, resident.1, package.package_number
    );
    add_log(db, "package_out", &log_action)?;
    
    Ok(format!("{} {}", resident.2, resident.3))
}

pub fn search_packages(db: &Connection, search_query: &str) -> Result<Vec<PackageData>, Error> {
    let query = format!("%{}%", search_query.to_lowercase());
    let mut stmt = db.prepare(
        "SELECT id, apt, package_number, tracking_number, date_time, status 
         FROM package 
         WHERE status = 'pending' AND (
            LOWER(apt) LIKE ?1 
            OR LOWER(package_number) LIKE ?1 
            OR LOWER(tracking_number) LIKE ?1
         )
         ORDER BY date_time DESC"
    )?;

    let query_map = stmt.query_map([&query], |row| {
        Ok(PackageData {
            id: row.get(0)?,
            apt: row.get(1)?,
            package_number: row.get(2)?,
            tracking_number: row.get(3)?,
            date_time: row.get(4)?,
            status: row.get(5)?,
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}

pub fn get_packages_for_resident(db: &Connection, apt: &str) -> Result<Vec<PackageData>, Error> {
    let mut query = db.prepare(
        "SELECT id, apt, package_number, tracking_number, date_time, status 
         FROM package 
         WHERE apt = ?1 AND status = 'pending'
         ORDER BY date_time DESC"
    )?;

    let query_map = query.query_map([apt], |row| {
        Ok(PackageData {
            id: row.get(0)?,
            apt: row.get(1)?,
            package_number: row.get(2)?,
            tracking_number: row.get(3)?,
            date_time: row.get(4)?,
            status: row.get(5)?,
        })
    })?;

    query_map.collect::<Result<Vec<_>, _>>()
}