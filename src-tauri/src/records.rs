//! Logical dictation records are independent of microphone sessions and Vosk endpoints.
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize)]
pub struct Fragment {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub id: u64,
    pub generation: u64,
    pub text: String,
    pub partial: String,
    pub fragments: Vec<Fragment>,
}

impl Record {
    fn new(id: u64) -> Self {
        Self {
            id,
            generation: 0,
            text: String::new(),
            partial: String::new(),
            fragments: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub version: u64,
    pub current: Record,
    pub history: Vec<Record>,
}

pub struct Records {
    current: Record,
    history: Vec<Record>,
    seen: HashSet<String>,
    version: u64,
}

#[cfg(test)]
mod tests;

pub fn append_text(target: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if !target.is_empty() && !target.ends_with('\n') {
        target.push('\n');
    }
    target.push_str(text);
}

impl Records {
    pub fn new() -> Self {
        Self {
            current: Record::new(1),
            history: Vec::new(),
            seen: HashSet::new(),
            version: 0,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: self.version,
            current: self.current.clone(),
            history: self.history.clone(),
        }
    }

    pub fn identity(&self) -> (u64, u64, u64) {
        (self.current.id, self.current.generation, self.version)
    }

    pub fn append(&mut self, id: String, text: String) -> bool {
        if text.trim().is_empty() || !self.seen.insert(id.clone()) {
            return false;
        }
        append_text(&mut self.current.text, &text);
        self.current.partial.clear();
        self.current.fragments.push(Fragment {
            id,
            text: text.trim().to_owned(),
        });
        true
    }

    pub fn partial(&mut self, text: String) {
        self.current.partial = text;
    }

    pub fn next(&mut self) {
        let next_id = self.current.id + 1;
        let previous = std::mem::replace(&mut self.current, Record::new(next_id));
        if !previous.text.is_empty() {
            self.history.push(previous);
        }
        self.version += 1;
    }

    pub fn clear(&mut self) -> Vec<String> {
        let ids = self
            .current
            .fragments
            .iter()
            .map(|part| part.id.clone())
            .collect();
        self.current.generation += 1;
        self.current.text.clear();
        self.current.partial.clear();
        self.current.fragments.clear();
        self.version += 1;
        ids
    }

    pub fn edit(
        &mut self,
        id: u64,
        generation: u64,
        text: String,
        base_ids: &[String],
    ) -> Result<Record, String> {
        let record = if self.current.id == id {
            Some(&mut self.current)
        } else {
            self.history.iter_mut().find(|entry| entry.id == id)
        }
        .ok_or("Запись уже недоступна")?;
        if record.generation != generation {
            return Err("Запись была очищена. Правка относится к предыдущему тексту.".into());
        }
        let base: HashSet<&str> = base_ids.iter().map(String::as_str).collect();
        // A hotkey may resume dictation while a manual edit is in flight.
        // Preserve final segments that arrived after the editor's captured draft.
        record.text = text;
        for fragment in &record.fragments {
            if !base.contains(fragment.id.as_str()) {
                append_text(&mut record.text, &fragment.text);
            }
        }
        Ok(record.clone())
    }
}
