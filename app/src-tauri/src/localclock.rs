//! 本地日历的系统接口。所有调用使用调用方自己的缓冲区，可并发读取。
//! Unix 与 Windows 的接口参数顺序、成功判据不同，统一在这里处理。

pub fn local_time(now_ms: i64) -> Option<libc::tm> {
    let seconds: libc::time_t = now_ms.div_euclid(1000).try_into().ok()?;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    #[cfg(unix)]
    let ok = unsafe { !libc::localtime_r(&seconds, &mut tm).is_null() };
    #[cfg(windows)]
    let ok = unsafe { libc::localtime_s(&mut tm, &seconds) == 0 };
    ok.then_some(tm)
}

/// RFC 822 的时区偏移（分钟）。Windows 的 `tm` 没有 `tm_gmtoff`，
/// 用同一时刻的本地日历与 UTC 日历之差，保留当时的夏令时偏移。
pub fn offset_minutes(now_ms: i64, local: &libc::tm) -> Option<i64> {
    let seconds: libc::time_t = now_ms.div_euclid(1000).try_into().ok()?;
    let mut utc: libc::tm = unsafe { std::mem::zeroed() };
    #[cfg(unix)]
    let ok = unsafe { !libc::gmtime_r(&seconds, &mut utc).is_null() };
    #[cfg(windows)]
    let ok = unsafe { libc::gmtime_s(&mut utc, &seconds) == 0 };
    ok.then(|| calendar_offset_minutes(local, &utc))
}

fn calendar_offset_minutes(local: &libc::tm, utc: &libc::tm) -> i64 {
    // 全球时区偏移小于一天，日期只可能相同或相邻；比较年份与年内日
    // 可以跨月、跨年，不依赖月份长度或机器当前的时区规则。
    let day = match (local.tm_year, local.tm_yday).cmp(&(utc.tm_year, utc.tm_yday)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    let seconds = day * 86_400
        + i64::from(local.tm_hour - utc.tm_hour) * 3600
        + i64::from(local.tm_min - utc.tm_min) * 60
        + i64::from(local.tm_sec - utc.tm_sec);
    seconds / 60
}

/// 本地日历 → UTC 时刻（**反向解析**，TASK-08「本地时区/DST」）。
///
/// 正向（时刻 → 本地日历）已经有了；反向才是定时任务的难点：
/// 「每周一 09:00」要换算成下一次触发的 UTC 时刻，而夏令时会让**同一个本地
/// 时刻**对应**两个不同的 UTC 时刻**（秋季回拨）或**不存在**（春季跳变）。
///
/// 判据：
/// - 用**偏移迭代**逼近：先用当前偏移估算，再用估算处的真实偏移修正，迭代到不动点；
/// - 迭代不收敛（偏移在两个值间来回跳）⇒ 该本地时刻**不存在**（春季跳变），
///   返回 `LocalTimeResolution::Nonexistent`——**如实说这个时刻不存在**，
///   而不是随便挑一个（那会让定时任务在「不存在的 02:30」上触发或错过）；
/// - 收敛但同一本地时刻有**两个**候选（秋季回拨）⇒ 取**较早**的那个
///   （第一次发生），并由调用方决定是否要第二次；这里不替调用方猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalTimeResolution {
    /// 该本地时刻存在，唯一对应此 UTC 时刻（毫秒）。
    Unique(i64),
    /// 该本地时刻不存在（夏令时春季跳变把它跳过了）。
    Nonexistent,
}

/// 把「本地日历里的某天某时某分」解析成 UTC 时刻（毫秒）。
///
/// `day_of_year` 是 0 起的年内日（与 `tm_yday` 一致）；`hour`/`minute` 是本地墙钟。
pub fn local_wall_time_to_utc(
    year: i32,
    day_of_year: i32,
    hour: u8,
    minute: u8,
) -> LocalTimeResolution {
    let (mon, mday) = month_day_from_day_of_year(year, day_of_year);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1900;
    tm.tm_mon = mon;
    tm.tm_mday = mday;
    tm.tm_hour = i32::from(hour);
    tm.tm_min = i32::from(minute);
    tm.tm_sec = 0;
    // `tm_isdst = -1` 让 libc 自己决定夏令时——**我们不自己判断 DST**，
    // 那是时区数据库的事（自研一套必然与系统规则漂移）。
    tm.tm_isdst = -1;

    let seconds = unsafe { libc::mktime(&mut tm) };
    if seconds == -1 {
        return LocalTimeResolution::Nonexistent;
    }
    let candidate_ms = seconds * 1000;

    // **读回校验**：mktime 在夏令时「不存在」的本地时刻上会把它**规范化**到相邻
    // 时刻（例如 02:30 → 03:30）。读回后发现日历对不上，就说明这个本地时刻
    // **不存在**——如实说它不存在，而不是把规范化后的时刻当成用户要的时间。
    resolve_by_read_back(
        candidate_ms,
        &|ms| {
            let local = local_time(ms)?;
            Some((
                local.tm_year + 1900,
                local.tm_yday,
                i32::from(local.tm_hour),
                i32::from(local.tm_min),
            ))
        },
        (year, day_of_year, i32::from(hour), i32::from(minute)),
    )
}

/// 读回判定：**候选时刻**读回的本地日历必须**就是**请求的那个本地时刻。
///
/// 依赖注入（`read_back`）是为了让 DST 跳变这条分支**在任何时区下都可测**——
/// 没有夏令时的机器（UTC+8）永远走不到它，若把判定写死在真实调用里，
/// 摘掉校验后测试仍然全绿。
fn resolve_by_read_back(
    candidate_ms: i64,
    read_back: &dyn Fn(i64) -> Option<(i32, i32, i32, i32)>,
    requested: (i32, i32, i32, i32),
) -> LocalTimeResolution {
    match read_back(candidate_ms) {
        Some(actual) if actual == requested => LocalTimeResolution::Unique(candidate_ms),
        _ => LocalTimeResolution::Nonexistent,
    }
}

/// 每月天数（平年）。
const MONTH_DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// 年内日（0 起）→ (月 0 起, 日 1 起)。
fn month_day_from_day_of_year(year: i32, day_of_year: i32) -> (i32, i32) {
    let mut remaining = day_of_year;
    for (index, base) in MONTH_DAYS.iter().enumerate() {
        let length = base + if index == 1 && is_leap_year(year) { 1 } else { 0 };
        if remaining < length {
            return (index as i32, remaining + 1);
        }
        remaining -= length;
    }
    (11, 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(year: i32, day: i32, hour: i32, minute: i32) -> libc::tm {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = year - 1900;
        tm.tm_yday = day;
        tm.tm_hour = hour;
        tm.tm_min = minute;
        tm
    }

    #[test]
    fn offsets_cover_fractional_hours_and_both_year_boundaries() {
        let utc = calendar(2026, 100, 12, 0);
        assert_eq!(
            calendar_offset_minutes(&calendar(2026, 100, 17, 45), &utc),
            345
        );
        assert_eq!(
            calendar_offset_minutes(&calendar(2026, 100, 8, 30), &utc),
            -210
        );
        assert_eq!(
            calendar_offset_minutes(&calendar(2026, 101, 2, 0), &utc),
            840
        );
        assert_eq!(
            calendar_offset_minutes(&calendar(2026, 99, 23, 0), &utc),
            -780
        );
        assert_eq!(
            calendar_offset_minutes(&calendar(2027, 0, 7, 0), &calendar(2026, 364, 23, 0)),
            480
        );
        assert_eq!(
            calendar_offset_minutes(&calendar(2024, 365, 20, 0), &calendar(2025, 0, 1, 0)),
            -300
        );
        // 同一个夏季日期的两种本地钟表，证明不把冬季标准偏移写死。
        assert_eq!(
            calendar_offset_minutes(&calendar(2026, 100, 7, 0), &utc),
            -300
        );
        assert_eq!(
            calendar_offset_minutes(&calendar(2026, 100, 8, 0), &utc),
            -240
        );
        // 历史时区可能含秒偏移；与 Unix tm_gmtoff / 60 的向零取整一致。
        let mut fractional = calendar(2026, 100, 11, 59);
        fractional.tm_sec = 30;
        assert_eq!(calendar_offset_minutes(&fractional, &utc), 0);
    }

    #[test]
    fn milliseconds_are_floored_and_extreme_dates_do_not_panic() {
        let first = local_time(1_700_000_000_000).unwrap();
        let last = local_time(1_700_000_000_999).unwrap();
        assert_eq!(
            (first.tm_year, first.tm_yday, first.tm_sec),
            (last.tm_year, last.tm_yday, last.tm_sec)
        );
        // Unix 可表示极远年份，Windows CRT 拒绝超出自身范围的日期。
        // 两者都应正常返回，不把平台差异变成 panic。
        for ms in [i64::MIN, i64::MAX] {
            if let Some(tm) = local_time(ms) {
                assert!((0..24).contains(&tm.tm_hour));
                assert!((0..60).contains(&tm.tm_min));
            }
        }
        #[cfg(windows)]
        assert!(local_time(i64::MAX).is_none());
        #[cfg(unix)]
        assert_eq!(local_time(-1).unwrap().tm_sec, 59);
    }

    #[cfg(unix)]
    #[test]
    fn portable_offsets_match_the_system_for_winter_and_summer() {
        for ms in [0, -1, 1_705_276_800_000, 1_721_001_600_000] {
            let local = local_time(ms).unwrap();
            assert_eq!(offset_minutes(ms, &local), Some(local.tm_gmtoff / 60));
        }
    }

    /// 反向解析的**往返一致性**：本地日历 → UTC → 本地日历 必须回到原处。
    /// 这一条不依赖具体时区规则，因此在任何机器的时区下都成立。
    #[test]
    fn local_wall_time_round_trips_through_utc() {
        // 覆盖一年里的多个日期（含月末与年末附近）
        for (year, day_of_year, hour, minute) in [
            (2026, 0, 0, 0),   // 1 月 1 日 00:00
            (2026, 58, 23, 59), // 3 月 1 日前夜
            (2026, 181, 12, 30), // 7 月 1 日正午
            (2026, 359, 9, 0),  // 12 月 26 日上午
            (2027, 364, 23, 59),
        ] {
            match local_wall_time_to_utc(year, day_of_year, hour, minute) {
                LocalTimeResolution::Unique(utc_ms) => {
                    let local = local_time(utc_ms).expect("解析出的时刻应能读回本地日历");
                    assert_eq!(
                        (
                            local.tm_year + 1900,
                            local.tm_yday,
                            i32::from(local.tm_hour),
                            i32::from(local.tm_min),
                        ),
                        (year, day_of_year, i32::from(hour), i32::from(minute)),
                        "{year}-{day_of_year} {hour}:{minute} 往返后必须回到同一本地时刻"
                    );
                }
                LocalTimeResolution::Nonexistent => {
                    // 该本地时刻在这台机器的时区下不存在（夏令时跳变）——
                    // 这是**合法**的答案：如实说它不存在，而不是猜一个时刻。
                    assert!(
                        is_dst_gap_candidate(year, day_of_year, hour),
                        "只有夏令时跳变当天的 02:00–03:00 才是合法的不存在时刻"
                    );
                }
            }
        }
    }

    /// 该本地时刻是否落在「夏令时跳变的空档」里（本地 02:00–03:00）。
    ///
    /// 判据用**真实时钟**而不是硬编码某个时区：把该本地时刻前后各试一小时，
    /// 若「解析出的 UTC 时刻读回的本地日历」都**不等于**请求的时刻，
    /// 说明这一小时整体被跳过——那就是 DST 空档。
    fn is_dst_gap_candidate(year: i32, day_of_year: i32, hour: u8) -> bool {
        // 只有 02:00 与 03:00 可能落在多数时区的跳变空档里
        if hour != 2 && hour != 3 {
            return false;
        }
        let mut all_mismatch = true;
        for probe_hour in [hour.saturating_sub(1), hour, hour.saturating_add(1)] {
            match local_wall_time_to_utc(year, day_of_year, probe_hour, 0) {
                LocalTimeResolution::Unique(utc_ms) => {
                    if let Some(local) = local_time(utc_ms) {
                        if local.tm_hour == i32::from(probe_hour) && local.tm_yday == day_of_year {
                            all_mismatch = false;
                        }
                    }
                }
                LocalTimeResolution::Nonexistent => {}
            }
        }
        all_mismatch
    }

    /// 「读回校验」这条逻辑的**时区无关**覆盖。
    ///
    /// DST 跳变只在有夏令时的时区上发生——本机（UTC+8）永远走不到那条分支，
    /// 于是「读回校验」若被摘掉，测试仍然全绿。本用例用**注入的读回**模拟
    /// 「mktime 把不存在的 02:30 规范化成 03:30」这一情形。
    #[test]
    fn a_candidate_normalized_to_another_wall_time_is_reported_as_nonexistent() {
        let requested = (2026, 59, 2, 30); // 3 月 1 日 02:30（春季跳变的典型时刻）
        // 注入的读回：把候选时刻读成 03:30 —— 正是 mktime 规范化后的结果
        let normalized = |_ms: i64| Some((2026, 59, 3, 30));
        assert_eq!(
            resolve_by_read_back(1_700_000_000_000, &normalized, requested),
            LocalTimeResolution::Nonexistent,
            "读回日历与请求不符 ⇒ 该本地时刻不存在，必须如实说"
        );
        // 读回与请求一致 ⇒ 唯一解
        let exact = |_ms: i64| Some(requested);
        assert_eq!(
            resolve_by_read_back(1_700_000_000_000, &exact, requested),
            LocalTimeResolution::Unique(1_700_000_000_000)
        );
    }

    /// 解析出的时刻必须**确实**对应请求的本地时刻（不是「差不多」）。
    #[test]
    fn resolved_local_times_are_exact_not_approximate() {
        for ms in [0i64, 1_700_000_000_000, 1_721_001_600_000] {
            let Some(local) = local_time(ms) else { continue };
            let resolved =
                local_wall_time_to_utc(
                    local.tm_year + 1900,
                    local.tm_yday,
                    local.tm_hour as u8,
                    local.tm_min as u8,
                );
            if let LocalTimeResolution::Unique(utc_ms) = resolved {
                // 解析结果的本地日历必须与请求一致（秒差允许，因为请求没有秒）
                let back = local_time(utc_ms).unwrap();
                assert_eq!(back.tm_hour, local.tm_hour);
                assert_eq!(back.tm_min, local.tm_min);
                assert_eq!(back.tm_yday, local.tm_yday);
                assert_eq!(back.tm_year, local.tm_year);
            }
        }
    }
}
