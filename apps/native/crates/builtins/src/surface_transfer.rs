//! Bounded host arguments; only complete surface transactions are observable.
use serde_json::Value;

#[derive(Default)]
pub(super) struct Transfer {
    text: String,
    next: usize,
}
impl Transfer {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn push(&mut self, input: &Value, limit: usize) -> Result<Option<Value>, String> {
        let result = self.append(input, limit);
        if result.is_err() || matches!(&result, Ok(Some(_))) {
            self.clear();
        }
        result
    }
    fn append(&mut self, input: &Value, limit: usize) -> Result<Option<Value>, String> {
        let index = input["index"].as_u64().ok_or("chunk index")?;
        if index == 0 {
            self.clear();
        }
        if index != self.next as u64 {
            return Err("out-of-order surface chunk".into());
        }
        let text = input["text"].as_str().ok_or("chunk text")?;
        let done = input["done"].as_bool().ok_or("chunk completion")?;
        if text.is_empty()
            || text.len() > 64_000
            || self.text.len().saturating_add(text.len()) > limit
        {
            return Err("surface transfer byte limit".into());
        }
        self.text.push_str(text);
        self.next += 1;
        if done {
            serde_json::from_str(&self.text)
                .map(Some)
                .map_err(|e| e.to_string())
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn timeline_chunks_are_atomic_and_keep_existing_total_limit() {
        let text = json!({"baseRevision":0,"ops":(0..1600).map(|id|
            json!({"op":"setText","id":id,"text":"😀".repeat(200)})).collect::<Vec<_>>()})
        .to_string();
        assert!(text.len() > 1024 * 1024);
        let mut transfer = Transfer::default();
        let mut starts: Vec<_> = text.char_indices().map(|(i, _)| i).step_by(4000).collect();
        starts.push(text.len());
        for (index, bounds) in starts.windows(2).enumerate() {
            let done = bounds[1] == text.len();
            let value = transfer
                .push(
                    &json!({"index":index,"text":&text[bounds[0]..bounds[1]],"done":done}),
                    16 * 1024 * 1024,
                )
                .unwrap();
            assert_eq!(value.is_some(), done);
            if done {
                assert_eq!(value.unwrap().to_string(), text);
            }
        }
        assert!(
            transfer
                .push(&json!({"index":0,"text":"1234","done":false}), 3)
                .is_err()
        );
        assert!(
            transfer
                .push(&json!({"index":1,"text":"{}","done":true}), 100)
                .is_err()
        );
        assert_eq!(
            transfer
                .push(&json!({"index":0,"text":"{}","done":true}), 100)
                .unwrap(),
            Some(json!({}))
        );
    }
}
