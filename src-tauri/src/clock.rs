//! 时间来源与"业务日期"。
//!
//! 所有"今天"的判断都必须调用 [`business_date`]（或 [`Clock::business_date`]），
//! 不要在各处自己用系统日期计算。时钟可注入，便于测试跨日、跨周、跨月。

use chrono::{Days, Local, NaiveDate, NaiveDateTime, NaiveTime};

use crate::error::{AppError, AppResult};

/// 当前时间的来源。正式运行用 [`SystemClock`]，测试用 [`FixedClock`]。
pub trait Clock: Send + Sync {
    /// 本地时间（不带时区）
    fn now(&self) -> NaiveDateTime;

    fn business_date(&self, cutoff: NaiveTime) -> NaiveDate {
        business_date(self.now(), cutoff)
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> NaiveDateTime {
        Local::now().naive_local()
    }
}

/// 固定时间的时钟，用于测试
pub struct FixedClock(pub NaiveDateTime);

impl Clock for FixedClock {
    fn now(&self) -> NaiveDateTime {
        self.0
    }
}

/// 业务日期：日切时间之前仍属于前一天。
///
/// 例：日切 04:00，则 10 月 18 日 03:59 的业务日期是 10 月 17 日。
pub fn business_date(now: NaiveDateTime, cutoff: NaiveTime) -> NaiveDate {
    let date = now.date();
    if now.time() < cutoff {
        date.checked_sub_days(Days::new(1)).unwrap_or(date)
    } else {
        date
    }
}

/// 某个业务日期开始的时刻（即当天的日切时刻）
pub fn business_day_start(date: NaiveDate, cutoff: NaiveTime) -> NaiveDateTime {
    date.and_time(cutoff)
}

/// 解析设置中的日切时间，格式 `HH:MM`
pub fn parse_cutoff(s: &str) -> AppResult<NaiveTime> {
    let valid = s.len() == 5 && s.as_bytes()[2] == b':';
    valid
        .then(|| NaiveTime::parse_from_str(s, "%H:%M").ok())
        .flatten()
        .ok_or_else(|| AppError::Invalid(format!("日切时间格式不正确：{s}（应为 HH:MM）")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }
    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn t(s: &str) -> NaiveTime {
        parse_cutoff(s).unwrap()
    }

    #[test]
    fn 零点日切时就是日历日期() {
        assert_eq!(
            business_date(at("2026-10-17 00:00:00"), t("00:00")),
            d("2026-10-17")
        );
        assert_eq!(
            business_date(at("2026-10-17 23:59:59"), t("00:00")),
            d("2026-10-17")
        );
    }

    #[test]
    fn 日切之前属于前一天() {
        let cutoff = t("04:00");
        assert_eq!(
            business_date(at("2026-10-18 03:59:59"), cutoff),
            d("2026-10-17")
        );
        assert_eq!(
            business_date(at("2026-10-18 04:00:00"), cutoff),
            d("2026-10-18")
        );
        assert_eq!(
            business_date(at("2026-10-18 00:00:00"), cutoff),
            d("2026-10-17")
        );
    }

    #[test]
    fn 跨月跨年与闰日() {
        let cutoff = t("04:00");
        assert_eq!(
            business_date(at("2026-11-01 01:00:00"), cutoff),
            d("2026-10-31")
        );
        assert_eq!(
            business_date(at("2027-01-01 03:00:00"), cutoff),
            d("2026-12-31")
        );
        assert_eq!(
            business_date(at("2028-03-01 02:00:00"), cutoff),
            d("2028-02-29")
        );
    }

    #[test]
    fn 业务日开始时刻() {
        assert_eq!(
            business_day_start(d("2026-10-17"), t("04:00")),
            at("2026-10-17 04:00:00")
        );
        // 业务日开始时刻本身属于该业务日
        let start = business_day_start(d("2026-10-17"), t("04:00"));
        assert_eq!(business_date(start, t("04:00")), d("2026-10-17"));
    }

    #[test]
    fn 可注入时钟() {
        let clock = FixedClock(at("2026-10-18 02:30:00"));
        assert_eq!(clock.business_date(t("04:00")), d("2026-10-17"));
        assert_eq!(clock.business_date(t("00:00")), d("2026-10-18"));
    }

    #[test]
    fn 日切时间格式校验() {
        assert!(parse_cutoff("04:00").is_ok());
        assert!(parse_cutoff("23:59").is_ok());
        for bad in ["24:00", "4:00", "04:60", "0400", "", "abc"] {
            assert!(parse_cutoff(bad).is_err(), "{bad} 应该无效");
        }
    }
}
