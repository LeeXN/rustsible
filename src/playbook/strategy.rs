use anyhow::{bail, Context, Result};
use serde_yaml::Value;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Strategy {
    #[default]
    Linear,
    Free,
}

#[derive(Default)]
pub(super) struct Throttles(Mutex<HashMap<String, Arc<Limit>>>);
struct Limit {
    maximum: usize,
    active: Mutex<usize>,
    available: Condvar,
}
pub(super) struct Permit(Arc<Limit>);
impl Throttles {
    pub fn acquire(&self, key: String, maximum: usize) -> Result<Permit> {
        if maximum == 0 {
            bail!("Throttle permit limit must be positive");
        }
        let limit = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Throttle registry was poisoned"))?
            .entry(key)
            .or_insert_with(|| {
                Arc::new(Limit {
                    maximum,
                    active: Mutex::new(0),
                    available: Condvar::new(),
                })
            })
            .clone();
        let mut active = limit
            .active
            .lock()
            .map_err(|_| anyhow::anyhow!("Throttle state was poisoned"))?;
        while *active >= limit.maximum {
            active = limit
                .available
                .wait(active)
                .map_err(|_| anyhow::anyhow!("Throttle state was poisoned"))?;
        }
        *active += 1;
        drop(active);
        Ok(Permit(limit))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        // Unwinding must release a slot too, so a failed host cannot leave its
        // peers parked behind a permit that will never become available.
        let mut active = self
            .0
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = active.saturating_sub(1);
        self.0.available.notify_one();
    }
}

/// Percentages always use the original play size. Once a serial list is
/// exhausted, its last size repeats rather than consuming the whole remainder.
pub(super) fn batches(serial: Option<&Value>, total: usize) -> Result<Vec<Range<usize>>> {
    let values = match serial {
        None => Vec::new(),
        Some(Value::Sequence(values)) => values.clone(),
        Some(value) => vec![value.clone()],
    };
    let mut sizes = Vec::new();
    for value in values {
        let count = match value {
            Value::Number(value) => value.as_i64().context("Serial count must be an integer")?,
            Value::String(value) => {
                if let Some(percent) = value.strip_suffix('%') {
                    let percent = percent
                        .parse::<u32>()
                        .context("Serial percentage must be an integer")?;
                    if percent > 100 {
                        bail!("Serial percentage must be between 0 and 100");
                    }
                    // Integer arithmetic avoids rounding changing a batch at a boundary.
                    let count = ((total as u128 * percent as u128) / 100).max(1);
                    i64::try_from(count).context("Serial batch exceeds the supported host count")?
                } else {
                    value
                        .parse::<i64>()
                        .context("Serial must be a count or percentage")?
                }
            }
            _ => bail!("Serial must be a count, percentage, or list of those values"),
        };
        sizes.push(if count <= 0 {
            total.max(1)
        } else {
            usize::try_from(count)?.min(total.max(1))
        });
    }
    if sizes.is_empty() {
        sizes.push(total.max(1));
    }
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < total {
        let size = sizes[result.len().min(sizes.len() - 1)];
        let end = offset.saturating_add(size).min(total);
        result.push(offset..end);
        offset = end;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_counts_percentages_and_lists_keep_original_denominator() {
        let serial: Value = serde_yaml::from_str("[1, '30%']").unwrap();
        assert_eq!(
            batches(Some(&serial), 10).unwrap(),
            vec![0..1, 1..4, 4..7, 7..10]
        );
        assert_eq!(
            batches(Some(&Value::String("0%".into())), 3).unwrap(),
            vec![0..1, 1..2, 2..3]
        );
        assert_eq!(
            batches(Some(&Value::Number(0.into())), 3).unwrap(),
            vec![0..3]
        );
        assert_eq!(batches(None, 0).unwrap(), Vec::<Range<usize>>::new());
        for input in ["true", "-1%", "101%", "[2, bad]", "1.5", "{size: 1}"] {
            assert!(
                batches(Some(&serde_yaml::from_str(input).unwrap()), 10).is_err(),
                "{input}"
            );
        }
    }
}
