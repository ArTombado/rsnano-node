use crate::utils::create_property_tree;
use anyhow::Result;
use chrono::{DateTime, Local};
use rsnano_core::utils::PropertyTree;
use std::{any::Any, fs::File, io::Write, path::PathBuf, time::SystemTime};

pub trait StatsLogSink {
    /// Called before logging starts
    fn begin(&mut self) -> Result<()>;

    /// Called after logging is completed
    fn finalize(&mut self);

    /// Write a header enrty to the log
    fn write_header(&mut self, header: &str, walltime: SystemTime) -> Result<()>;

    /// Write a counter or sampling entry to the log. Some log sinks may support writing histograms as well.
    fn write_counter_entry(
        &mut self,
        time: SystemTime,
        entry_type: &str,
        detail: &str,
        dir: &str,
        value: u64,
    ) -> Result<()>;

    fn write_sampler_entry(
        &mut self,
        time: SystemTime,
        sample: &str,
        values: Vec<i64>,
    ) -> Result<()>;

    /// Rotates the log (e.g. empty file). This is a no-op for sinks where rotation is not supported.
    fn rotate(&mut self) -> Result<()>;

    /// Returns a reference to the log entry counter
    fn entries(&self) -> usize;

    fn inc_entries(&mut self);

    /// Returns the string representation of the log. If not supported, an empty string is returned.
    fn to_string(&self) -> String;

    /// Returns the object representation of the log result. The type depends on the sink used.
    /// returns Object, or nullptr if no object result is available.
    fn to_object(&self) -> Option<&dyn Any>;
}

/// File sink with rotation support. This writes one counter per line and does not include histogram values.
pub struct StatFileWriter {
    filename: PathBuf,
    file: File,
    log_entries: usize,
}

impl StatFileWriter {
    pub fn new(filename: impl Into<PathBuf>) -> Result<Self> {
        let filename = filename.into();
        let file = File::create(filename.clone())?;
        Ok(Self {
            filename,
            file,
            log_entries: 0,
        })
    }
}

impl StatsLogSink for StatFileWriter {
    fn begin(&mut self) -> Result<()> {
        Ok(())
    }

    fn finalize(&mut self) {}

    fn write_header(&mut self, header: &str, walltime: SystemTime) -> Result<()> {
        let local = DateTime::<Local>::from(walltime);
        let local_fmt = local.format("%Y.%m.%d %H:%M:%S");
        writeln!(&mut self.file, "{header},{local_fmt}")?;
        Ok(())
    }

    fn write_counter_entry(
        &mut self,
        time: SystemTime,
        entry_type: &str,
        detail: &str,
        dir: &str,
        value: u64,
    ) -> Result<()> {
        let now = DateTime::<Local>::from(time).format("%H:%M:%S");
        writeln!(&mut self.file, "{now},{entry_type},{detail},{dir},{value}")?;
        Ok(())
    }

    fn write_sampler_entry(
        &mut self,
        time: SystemTime,
        sample: &str,
        values: Vec<i64>,
    ) -> Result<()> {
        let time: chrono::DateTime<Local> = time.into();
        write!(&mut self.file, "{},{sample}", time.format("%H:%M:%S"))?;

        for value in values {
            write!(&mut self.file, ",{}", value)?;
        }

        writeln!(&mut self.file, "")?;

        Ok(())
    }

    fn rotate(&mut self) -> Result<()> {
        self.file = File::create(self.filename.clone())?;
        self.log_entries = 0;
        Ok(())
    }

    fn entries(&self) -> usize {
        self.log_entries
    }

    fn inc_entries(&mut self) {
        self.log_entries += 1;
    }

    fn to_string(&self) -> String {
        String::new()
    }

    fn to_object(&self) -> Option<&dyn Any> {
        None
    }
}

/// JSON sink. The resulting JSON object is provided as both a property_tree::ptree (to_object) and a string (to_string)
pub struct StatsJsonWriter {
    tree: Box<dyn PropertyTree>,
    entries_tree: Box<dyn PropertyTree>,
    log_entries: usize,
}

impl StatsJsonWriter {
    pub fn new() -> Self {
        Self {
            tree: create_property_tree(),
            entries_tree: create_property_tree(),
            log_entries: 0,
        }
    }
}

impl Default for StatsJsonWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl StatsLogSink for StatsJsonWriter {
    fn begin(&mut self) -> Result<()> {
        self.tree.clear()
    }

    fn finalize(&mut self) {
        self.tree.add_child("entries", self.entries_tree.as_ref());
    }

    fn write_header(&mut self, header: &str, walltime: SystemTime) -> Result<()> {
        let now = DateTime::<Local>::from(walltime);
        self.tree.put_string("type", header)?;
        self.tree
            .put_string("created", &now.format("%Y.%m.%d %H:%M:%S").to_string())?;
        Ok(())
    }

    fn write_counter_entry(
        &mut self,
        time: SystemTime,
        entry_type: &str,
        detail: &str,
        dir: &str,
        value: u64,
    ) -> Result<()> {
        let mut entry = create_property_tree();
        entry.put_string(
            "time",
            &DateTime::<Local>::from(time).format("%H:%M:%S").to_string(),
        )?;
        entry.put_string("type", entry_type)?;
        entry.put_string("detail", detail)?;
        entry.put_string("dir", dir)?;
        entry.put_u64("value", value)?;
        self.entries_tree.push_back("", entry.as_ref());
        Ok(())
    }

    fn rotate(&mut self) -> Result<()> {
        Ok(())
    }

    fn entries(&self) -> usize {
        self.log_entries
    }

    fn inc_entries(&mut self) {
        self.log_entries += 1;
    }

    fn to_string(&self) -> String {
        self.tree.to_json()
    }

    fn to_object(&self) -> Option<&dyn Any> {
        Some(self.tree.as_ref().as_any())
    }

    fn write_sampler_entry(
        &mut self,
        time: SystemTime,
        sample: &str,
        values: Vec<i64>,
    ) -> Result<()> {
        let time: chrono::DateTime<Local> = time.into();
        let mut entry = create_property_tree();
        entry.put_string("time", &time.format("%H:%M:%S").to_string())?;
        entry.put_string("sample", sample)?;

        let mut values_tree = create_property_tree();
        for value in values {
            let mut value_tree = create_property_tree();
            value_tree.put_string("", &value.to_string())?;
            values_tree.push_back("", value_tree.as_ref());
        }
        entry.add_child("values", values_tree.as_ref());
        self.entries_tree.push_back("", entry.as_ref());
        Ok(())
    }
}
