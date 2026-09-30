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
}
