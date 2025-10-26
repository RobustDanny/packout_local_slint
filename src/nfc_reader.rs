use pcsc::*;
use std::thread;
use std::time::Duration;
use std::ffi::CString;
use sha2::{Sha256, Digest};

pub struct NFCReader {
    context: Context,
    reader_name: Option<CString>,
}

#[derive(Debug, Clone)]
pub struct NFCCardData {
    pub uid: String,
    pub resident_id: u32,
    pub apt: String,
    pub added_date: String,
}

impl NFCCardData {
    pub fn generate_hash(&self) -> String {
        let data_to_hash = format!(
            "{}:{}:{}:{}",
            self.uid,
            self.resident_id,
            self.apt,
            self.added_date
        );
        
        let mut hasher = Sha256::new();
        hasher.update(data_to_hash.as_bytes());
        let result = hasher.finalize();
        
        format!("{:x}", result)
    }
}

impl NFCReader {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let context = Context::establish(Scope::User)?;
        Ok(NFCReader {
            context,
            reader_name: None,
        })
    }

    pub fn list_readers(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut readers_buf = [0; 2048];
        let readers = self.context.list_readers(&mut readers_buf)?;
        
        let reader_names: Vec<String> = readers
            .map(|r| r.to_str().unwrap_or("Unknown").to_string())
            .collect();
        
        Ok(reader_names)
    }

    pub fn select_reader(&mut self, reader_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let c_reader_name = CString::new(reader_name)?;
        self.reader_name = Some(c_reader_name);
        Ok(())
    }

    /// Control LED on NFC reader (for ACR122U and compatible readers)
    /// - color: 1 = Green, 2 = Red, 3 = Orange/Both
    /// - duration_ms: How long to light (0 = permanent until next command)
    pub fn set_led(&self, color: u8, duration_ms: u16) -> Result<(), Box<dyn std::error::Error>> {
        if self.reader_name.is_none() {
            return Err("No reader selected".into());
        }

        let reader_name = self.reader_name.as_ref().unwrap();
        
        let card = self.context.connect(
            reader_name.as_c_str(),
            ShareMode::Shared,
            Protocols::ANY,
        )?;

        // ACR122U LED control command
        // FF 00 40 [LED_STATE] 04 [T1] [T2] [REPEAT] [LINK]
        // LED_STATE: bits control red/green LEDs
        //   Bit 0-3: Green LED state
        //   Bit 4-7: Red LED state
        //   Values: 0=off, 1=on, 2=blink
        
        let led_state = match color {
            1 => 0x01, // Green only
            2 => 0x10, // Red only
            3 => 0x11, // Both (orange)
            _ => 0x00, // Off
        };

        // T1/T2: duration in units of 100ms (max 255 = 25.5 seconds)
        let duration_units = ((duration_ms / 100) as u8).min(255);
        
        let apdu_led_control = [
            0xFF, 0x00, 0x40, led_state, 0x04,
            duration_units, // T1: Initial blink duration
            duration_units, // T2: Toggle blink duration  
            0x01,           // Repeat: number of cycles
            0x00,           // Link to buzzer (0=no link)
        ];

        let mut response_buf = [0; MAX_BUFFER_SIZE];
        
        match card.transmit(&apdu_led_control, &mut response_buf) {
            Ok(resp) => {
                if resp.len() >= 2 && resp[resp.len()-2] == 0x90 && resp[resp.len()-1] == 0x00 {
                    println!("✅ LED set to color {} for {}ms", color, duration_ms);
                }
            }
            Err(e) => {
                println!("⚠️  LED control not supported by this reader: {}", e);
                // Not a fatal error - continue without LED
            }
        }
        
        let _ = card.disconnect(Disposition::LeaveCard);
        Ok(())
    }

    /// Convenience method: Flash green LED to indicate success
    pub fn signal_success(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("💚 Signaling success with green LED");
        self.set_led(1, 2000) // Green for 2 seconds
    }

    /// Convenience method: Flash red LED to indicate error
    pub fn signal_error(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("❌ Signaling error with red LED");
        self.set_led(2, 2000) // Red for 2 seconds
    }

    /// Convenience method: Flash orange LED to indicate warning/processing
    pub fn signal_processing(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("🟠 Signaling processing with orange LED");
        self.set_led(3, 1000) // Orange for 1 second
    }

    pub fn read_card_uid(&self) -> Result<String, Box<dyn std::error::Error>> {
        if self.reader_name.is_none() {
            return Err("No reader selected".into());
        }

        let reader_name = self.reader_name.as_ref().unwrap();
        
        let card = self.context.connect(
            reader_name.as_c_str(),
            ShareMode::Shared,
            Protocols::ANY,
        )?;

        let mut atr_buf = [0; MAX_ATR_SIZE];
        let mut reader_names_buf = [0; 256];
        
        let _status = match card.status2(&mut reader_names_buf, &mut atr_buf) {
            Ok(status) => status,
            Err(e) => {
                let _ = card.disconnect(Disposition::LeaveCard);
                return Err(e.into());
            }
        };
        
        let apdu_get_uid = [0xFF, 0xCA, 0x00, 0x00, 0x00];
        let mut response_buf = [0; MAX_BUFFER_SIZE];
        
        let response = match card.transmit(&apdu_get_uid, &mut response_buf) {
            Ok(resp) => resp,
            Err(e) => {
                let _ = card.disconnect(Disposition::LeaveCard);
                return Err(e.into());
            }
        };
        
        let uid = response[..response.len()-2]
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<String>>()
            .join("");
        
        let _ = card.disconnect(Disposition::LeaveCard);
        
        Ok(uid)
    }

    pub fn wait_for_card(&self, timeout_secs: u64) -> Result<String, Box<dyn std::error::Error>> {
        if self.reader_name.is_none() {
            return Err("No reader selected".into());
        }

        let reader_name = self.reader_name.as_ref().unwrap();
        let start = std::time::Instant::now();
        
        loop {
            if start.elapsed().as_secs() > timeout_secs {
                return Err("Timeout waiting for card".into());
            }

            match self.context.connect(
                reader_name.as_c_str(),
                ShareMode::Shared,
                Protocols::ANY,
            ) {
                Ok(card) => {
                    let apdu_get_uid = [0xFF, 0xCA, 0x00, 0x00, 0x00];
                    let mut response_buf = [0; MAX_BUFFER_SIZE];
                    
                    match card.transmit(&apdu_get_uid, &mut response_buf) {
                        Ok(response) => {
                            let uid = response[..response.len()-2]
                                .iter()
                                .map(|b| format!("{:02X}", b))
                                .collect::<Vec<String>>()
                                .join("");
                            
                            let _ = card.disconnect(Disposition::LeaveCard);
                            return Ok(uid);
                        }
                        Err(_) => {
                            let _ = card.disconnect(Disposition::LeaveCard);
                            thread::sleep(Duration::from_millis(200));
                            continue;
                        }
                    }
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(200));
                    continue;
                }
            }
        }
    }

    pub fn write_hash_to_card(&self, hash: &str, _start_block: u8) -> Result<(), Box<dyn std::error::Error>> {
        if self.reader_name.is_none() {
            return Err("No reader selected".into());
        }

        println!("  🔍 Hash to write: '{}'", hash);
        println!("  🔍 Hash length: {} chars", hash.len());
        
        let hash_bytes: Vec<u8> = hash.as_bytes().to_vec();
        println!("  🔍 Hash as ASCII bytes: {:02X?}", hash_bytes);
        println!("  🔍 Byte count: {}", hash_bytes.len());

        let reader_name = self.reader_name.as_ref().unwrap();
        let start_page = 5u8;
        let mut byte_position = 0;
        
        // Signal processing started
        let _ = self.signal_processing();
        
        // Write each page with full connect-write-disconnect cycle
        for page_offset in 0..16 {
            let page = start_page + page_offset;
            let start_idx = byte_position;
            let end_idx = std::cmp::min(start_idx + 4, hash_bytes.len());
            
            let mut data_to_write = [0u8; 4];
            let chunk_bytes = &hash_bytes[start_idx..end_idx];
            data_to_write[..chunk_bytes.len()].copy_from_slice(chunk_bytes);

            println!("  ✍️  Page {}: writing {} bytes", page, chunk_bytes.len());
            println!("      Data bytes: {:02X?}", data_to_write);
            
            let mut retries = 5;
            let mut page_written = false;
            
            while retries > 0 && !page_written {
                if page_offset > 0 || retries < 5 {
                    thread::sleep(Duration::from_millis(500));
                }
                
                let card = match self.context.connect(
                    reader_name.as_c_str(),
                    ShareMode::Exclusive,
                    Protocols::ANY,
                ) {
                    Ok(card) => card,
                    Err(e) => {
                        println!("      ⚠️  Connect failed: {} (retry {} left)", e, retries - 1);
                        retries -= 1;
                        continue;
                    }
                };
                
                let mut response_buf = [0; MAX_BUFFER_SIZE];
                let mut compat_write_apdu = vec![0xFF, 0x00, 0x00, 0x00, 0x06, 0xA2, page];
                compat_write_apdu.extend_from_slice(&data_to_write);
                
                println!("      📤 APDU: {:02X?}", compat_write_apdu);
                
                let write_result = card.transmit(&compat_write_apdu, &mut response_buf);
                let _ = card.disconnect(Disposition::LeaveCard);
                
                match write_result {
                    Ok(resp) => {
                        println!("      📥 Response: {:02X?}", &resp[..std::cmp::min(resp.len(), 10)]);
                        
                        if resp.len() >= 2 {
                            let sw1 = resp[resp.len() - 2];
                            let sw2 = resp[resp.len() - 1];
                            
                            if sw1 == 0x90 && sw2 == 0x00 {
                                println!("      ✅ Write OK");
                                page_written = true;
                                thread::sleep(Duration::from_millis(200));
                            } else if sw1 == 0x63 && sw2 == 0x00 {
                                println!("      ⚠️  Write completed with warning: SW={:02X}{:02X}", sw1, sw2);
                                page_written = true;
                                thread::sleep(Duration::from_millis(200));
                            } else {
                                println!("      ❌ Write failed: SW={:02X}{:02X}", sw1, sw2);
                                retries -= 1;
                            }
                        }
                    }
                    Err(e) => {
                        println!("      ❌ Write error: {}", e);
                        retries -= 1;
                    }
                }
            }
            
            if !page_written {
                let _ = self.signal_error();
                return Err(format!("Failed to write page {} after {} attempts. Card may be write-protected or incompatible.", page, 5).into());
            }
            
            byte_position += 4;
        }
        
        println!("  ✅ All {} pages written successfully", 16);
        
        // Final verification
        println!("  🔍 Verifying written data...");
        thread::sleep(Duration::from_millis(1500));
        
        match self.read_hash_from_card(0) {
            Ok(read_hash) => {
                let read_trimmed = read_hash.trim_end_matches('\0');
                let hash_trimmed = hash.trim_end_matches('\0');
                
                if read_trimmed == hash_trimmed {
                    println!("  ✅ Verification successful!");
                    println!("     Written: {} chars", hash_trimmed.len());
                    
                    // 🟢 GREEN LIGHT ON SUCCESS!
                    let _ = self.signal_success();
                    
                    Ok(())
                } else {
                    println!("  ⚠️  Hash mismatch!");
                    let _ = self.signal_error();
                    
                    if read_trimmed.starts_with(&hash_trimmed[..std::cmp::min(8, hash_trimmed.len())]) {
                        Err("Partial write detected - some pages may not have written correctly".into())
                    } else {
                        Err("Hash verification failed - data mismatch".into())
                    }
                }
            }
            Err(e) => {
                println!("  ⚠️  Verification read failed: {}", e);
                let _ = self.signal_error();
                Ok(())
            }
        }
    }

    pub fn read_hash_from_card(&self, _start_block: u8) -> Result<String, Box<dyn std::error::Error>> {
        if self.reader_name.is_none() {
            return Err("No reader selected".into());
        }
    
        let reader_name = self.reader_name.as_ref().unwrap();
        let card = self.context.connect(
            reader_name.as_c_str(),
            ShareMode::Shared,
            Protocols::ANY,
        )?;
    
        let mut response_buf = [0; MAX_BUFFER_SIZE];
        let mut hash_bytes = Vec::new();
        let start_page = 5u8;
        
        for page_offset in 0..16 {
            let page = start_page + page_offset;
            let read_apdu = [0xFF, 0xB0, 0x00, page, 0x04];
            let response = card.transmit(&read_apdu, &mut response_buf)?;
            
            if response.len() < 6 {
                let _ = card.disconnect(Disposition::LeaveCard);
                return Err(format!("Page {} insufficient data", page).into());
            }
            
            let page_data = &response[..4];
            hash_bytes.extend_from_slice(page_data);
            println!("📖 Page {}: {:02X?}", page, page_data);
        }
        
        let _ = card.disconnect(Disposition::LeaveCard);
        
        let hash_bytes_trimmed: Vec<u8> = hash_bytes.iter()
            .take_while(|&&b| b != 0)
            .copied()
            .collect();
        
        let full_hash = String::from_utf8(hash_bytes_trimmed)
            .unwrap_or_else(|_| String::new());
        
        println!("📚 Full hash: '{}' (len: {})", full_hash, full_hash.len());
        Ok(full_hash)
    }

    // pub fn verify_card(&self, stored_hash: &str, block: u8) -> Result<bool, Box<dyn std::error::Error>> {
    //     let card_hash = self.read_hash_from_card(block)?;
    //     let is_valid = card_hash == stored_hash;
        
    //     // Signal result with LED
    //     if is_valid {
    //         let _ = self.signal_success();
    //     } else {
    //         let _ = self.signal_error();
    //     }
        
    //     Ok(is_valid)
    // }

    // pub fn is_card_present(&self) -> Result<bool, Box<dyn std::error::Error>> {
    //     if self.reader_name.is_none() {
    //         return Err("No reader selected".into());
    //     }

    //     let reader_name = self.reader_name.as_ref().unwrap();
        
    //     match self.context.connect(
    //         reader_name.as_c_str(),
    //         ShareMode::Shared,
    //         Protocols::ANY,
    //     ) {
    //         Ok(card) => {
    //             let _ = card.disconnect(Disposition::LeaveCard);
    //             Ok(true)
    //         }
    //         Err(_) => Ok(false),
    //     }
    // }
    
    pub fn force_disconnect(&self) -> Result<(), Box<dyn std::error::Error>> {
        if self.reader_name.is_none() {
            return Ok(());
        }

        let reader_name = self.reader_name.as_ref().unwrap();
        
        if let Ok(card) = self.context.connect(
            reader_name.as_c_str(),
            ShareMode::Shared,
            Protocols::ANY,
        ) {
            let _ = card.disconnect(Disposition::ResetCard);
        }
        
        Ok(())
    }
}

// pub fn start_card_verification_monitor<F>(
//     reader_name: String,
//     verification_callback: F,
// ) -> Result<(), Box<dyn std::error::Error>>
// where
//     F: Fn(String, String) + Send + 'static,
// {
//     thread::spawn(move || {
//         let mut reader = NFCReader::new().expect("Failed to create NFC reader");
//         reader.select_reader(&reader_name).expect("Failed to select reader");
        
//         let mut last_uid: Option<String> = None;
        
//         loop {
//             match reader.read_card_uid() {
//                 Ok(uid) => {
//                     if last_uid.as_ref() != Some(&uid) {
//                         match reader.read_hash_from_card(4) {
//                             Ok(hash) => {
//                                 verification_callback(uid.clone(), hash);
//                             }
//                             Err(_) => {
//                                 verification_callback(uid.clone(), String::new());
//                             }
//                         }
//                         last_uid = Some(uid);
//                     }
//                 }
//                 Err(_) => {
//                     last_uid = None;
//                 }
//             }
            
//             thread::sleep(Duration::from_millis(500));
//         }
//     });
    
//     Ok(())
// }