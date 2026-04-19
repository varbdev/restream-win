fn parse_u64_attr(line: &str, attr: &str) -> Option<u64> {
    let search = format!("{}=\"", attr);
    let start = line.find(&search)? + search.len();
    let end = line[start..].find('"')? + start;
    line[start..end].parse().ok()
}

fn rewrite_u64_attr(line: &str, attr: &str, new_val: u64) -> String {
    let search = format!("{}=\"", attr);
    let Some(attr_start) = line.find(&search) else {
        return line.to_string();
    };
    let val_start = attr_start + search.len();
    let Some(val_end_rel) = line[val_start..].find('"') else {
        return line.to_string();
    };
    let val_end = val_start + val_end_rel;
    format!("{}{}{}", &line[..val_start], new_val, &line[val_end..])
}

fn remove_r_attr(line: &str) -> String {
    let search = " r=\"";
    let Some(start) = line.find(search) else {
        return line.to_string();
    };
    let val_start = start + search.len();
    let Some(val_end_rel) = line[val_start..].find('"') else {
        return line.to_string();
    };
    let val_end = val_start + val_end_rel + 1;
    format!("{}{}", &line[..start], &line[val_end..])
}

pub fn trim_segment_timelines(xml: &str, min_video_seq: u64, min_audio_seq: u64) -> String {
    if min_video_seq == 0 && min_audio_seq == 0 {
        return xml.to_string();
    }

    let mut result = String::with_capacity(xml.len());
    let mut in_video_adapt = false;
    let mut in_audio_adapt = false;
    let mut trim_active = false;
    let mut new_start: u64 = 0;
    let mut seg_cursor: u64 = 0;

    for line in xml.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("<AdaptationSet") {
            in_video_adapt = trimmed.contains("video/mp4");
            in_audio_adapt = trimmed.contains("audio/mp4");
            trim_active = false;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if trimmed.starts_with("</AdaptationSet") {
            in_video_adapt = false;
            in_audio_adapt = false;
            trim_active = false;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        let in_trackable = in_video_adapt || in_audio_adapt;
        let min_seq = if in_video_adapt {
            min_video_seq
        } else if in_audio_adapt {
            min_audio_seq
        } else {
            0
        };

        if in_trackable && trimmed.starts_with("<SegmentTemplate") {
            let sn = parse_u64_attr(trimmed, "startNumber").unwrap_or(0);
            seg_cursor = sn;

            if min_seq > 0 && sn <= min_seq {
                trim_active = true;
                new_start = min_seq + 1;
                let new_line = rewrite_u64_attr(line, "startNumber", new_start);
                result.push_str(&new_line);
                result.push('\n');
            } else {
                trim_active = false;
                result.push_str(line);
                result.push('\n');
            }
            continue;
        }

        if in_trackable && trim_active && trimmed.starts_with("<S ") && trimmed.ends_with("/>") {
            let t = parse_u64_attr(trimmed, "t").unwrap_or(0);
            let d = parse_u64_attr(trimmed, "d").unwrap_or(0);
            let r = parse_u64_attr(trimmed, "r").unwrap_or(0);
            let count = r + 1;

            let skip_count = if new_start > seg_cursor {
                (new_start - seg_cursor).min(count)
            } else {
                0
            };

            seg_cursor += count;

            if skip_count >= count {
                continue;
            }

            if skip_count == 0 {
                result.push_str(line);
                result.push('\n');
            } else {
                let new_t = t + skip_count * d;
                let new_r = r.saturating_sub(skip_count);
                let mut new_line = rewrite_u64_attr(line, "t", new_t);
                if new_r == 0 {
                    new_line = remove_r_attr(&new_line);
                } else {
                    new_line = rewrite_u64_attr(&new_line, "r", new_r);
                }
                result.push_str(&new_line);
                result.push('\n');
            }
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    result
}
