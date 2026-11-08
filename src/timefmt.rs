//! Parsing and formatting durations.

/// Parse "25m", "1h30m", "90s", "1.5h", "45" (minutes) or "1:30:00" into seconds.
pub fn parse_duration(s: &str) -> Result<f64, String> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return Err("empty duration".into());
    }
    if let Ok(m) = s.parse::<f64>() {
        return if m > 0.0 { Ok(m * 60.0) } else { Err(format!("duration must be positive: '{s}'")) };
    }
    if s.contains(':') {
        let parts = s
            .split(':')
            .map(|p| p.parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| format!("bad duration '{s}'"))?;
        return match parts.as_slice() {
            [m, sec] => Ok(m * 60.0 + sec),
            [h, m, sec] => Ok(h * 3600.0 + m * 60.0 + sec),
            _ => Err(format!("bad duration '{s}'")),
        };
    }
    let mut total = 0.0;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let unit = match c {
            'h' => 3600.0,
            'm' => 60.0,
            's' => 1.0,
            _ => return Err(format!("unknown unit '{c}' in '{s}' (use h, m or s)")),
        };
        let v: f64 = num.parse().map_err(|_| format!("bad duration '{s}'"))?;
        total += v * unit;
        num.clear();
    }
    if !num.is_empty() {
        return Err(format!("missing unit after '{num}' in '{s}' (use h, m or s)"));
    }
    if total <= 0.0 {
        return Err(format!("duration must be positive: '{s}'"));
    }
    Ok(total)
}

/// 00:42, 12:05, 1:02:03
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        assert_eq!(parse_duration("25m"), Ok(1500.0));
        assert_eq!(parse_duration("1h30m"), Ok(5400.0));
        assert_eq!(parse_duration("90s"), Ok(90.0));
        assert_eq!(parse_duration("1.5h"), Ok(5400.0));
        assert_eq!(parse_duration("45"), Ok(2700.0));
        assert_eq!(parse_duration("1:30:00"), Ok(5400.0));
        assert_eq!(parse_duration("10:00"), Ok(600.0));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("10x").is_err());
        assert!(parse_duration("1h30").is_err());
        assert!(parse_duration("0").is_err());
    }

    #[test]
    fn formats_clock() {
        assert_eq!(clock(42.0), "00:42");
        assert_eq!(clock(725.0), "12:05");
        assert_eq!(clock(3723.0), "1:02:03");
    }
}
