use orts::tle::Tle;

/// Try fetching a TLE by NORAD catalog number. Tries CelesTrak first, falls back to SatNOGS.
pub fn try_fetch_tle_by_norad_id(norad_id: u32) -> Option<Tle> {
    if let Some(tle) = fetch_tle_celestrak(norad_id) {
        return Some(tle);
    }
    eprintln!("CelesTrak failed, trying SatNOGS...");
    fetch_tle_satnogs(norad_id)
}

/// Fetch a TLE by NORAD catalog number, panicking on failure.
pub fn fetch_tle_by_norad_id(norad_id: u32) -> Tle {
    try_fetch_tle_by_norad_id(norad_id)
        .unwrap_or_else(|| panic!("Failed to fetch TLE for NORAD ID {norad_id} from any source"))
}

/// Try fetching TLE from CelesTrak (3LE format).
fn fetch_tle_celestrak(norad_id: u32) -> Option<Tle> {
    let url = format!("https://celestrak.org/NORAD/elements/gp.php?CATNR={norad_id}&FORMAT=3LE");
    eprintln!("Fetching TLE for NORAD ID {norad_id} from CelesTrak...");
    let body = match ureq::get(&url).call() {
        Ok(mut resp) => match resp.body_mut().read_to_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to read CelesTrak response: {e}");
                return None;
            }
        },
        Err(e) => {
            eprintln!("Failed to fetch TLE from CelesTrak: {e}");
            return None;
        }
    };
    if body.trim().is_empty() {
        eprintln!("No TLE data found on CelesTrak for NORAD ID {norad_id}");
        return None;
    }
    match Tle::parse(&body) {
        Ok(tle) => Some(tle),
        Err(e) => {
            eprintln!("Failed to parse CelesTrak TLE: {e}");
            None
        }
    }
}

/// Try fetching TLE from SatNOGS DB (JSON API).
fn fetch_tle_satnogs(norad_id: u32) -> Option<Tle> {
    let url = format!("https://db.satnogs.org/api/tle/?norad_cat_id={norad_id}&format=json");
    eprintln!("Fetching TLE for NORAD ID {norad_id} from SatNOGS...");
    let body = match ureq::get(&url).call() {
        Ok(mut resp) => match resp.body_mut().read_to_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to read SatNOGS response: {e}");
                return None;
            }
        },
        Err(e) => {
            eprintln!("Failed to fetch TLE from SatNOGS: {e}");
            return None;
        }
    };
    let entries: Vec<serde_json::Value> = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to parse SatNOGS JSON: {e}");
            return None;
        }
    };
    let entry = entries.first()?;
    let tle0 = entry["tle0"].as_str().unwrap_or("");
    let tle1 = entry["tle1"].as_str()?;
    let tle2 = entry["tle2"].as_str()?;
    let tle_text = format!("{tle0}\n{tle1}\n{tle2}");
    match Tle::parse(&tle_text) {
        Ok(tle) => Some(tle),
        Err(e) => {
            eprintln!("Failed to parse SatNOGS TLE: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires network access
    fn fetch_iss_tle_from_celestrak() {
        let tle = fetch_tle_celestrak(25544);
        assert!(tle.is_some());
    }

    #[test]
    #[ignore] // Requires network access
    fn fetch_iss_tle_satnogs_fallback() {
        let tle = fetch_tle_satnogs(25544);
        assert!(tle.is_some());
    }
}
