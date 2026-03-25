use anyhow::Context as _;
use bevy::prelude::*;
use std::fs::File;
use std::path::Path;

#[derive(Resource)]
pub struct FileHandles {
    pub queue_stats: csv::Writer<File>,
}

pub fn setup() -> anyhow::Result<FileHandles> {
    let now = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");

    // Create folder
    std::fs::create_dir_all("./data").context("create data dir")?;
    // Create files

    let _mmr_file = format!("./data/{}-mmr.csv", now);
    let queue_stats_file = format!("./data/{}-queue_stats.csv", now);
    let queue_stats_path = std::path::Path::new(&queue_stats_file);
    File::create(queue_stats_path).context("create file")?;
    let mut queue_stats_writer = csv::Writer::from_path(queue_stats_path).context("create csv writer")?;
    queue_stats_writer.write_record([
        "Mean Wait Time",
        "Mean MMR Range",
        "Players In Queue",
        "Players In Match",
        "Total Players in Pool",
        "Players Logged Out",
        "Mean MMR in Pool",
        "Median MMR in Pool",
        "Highest MMR in Pool",
        "Matches Played by Highest MMR",
        "Lowest MMR in Pool",
        "Matches Played by Lowest MMR",
    ])?;

    Ok(FileHandles {
        queue_stats: queue_stats_writer,
    })
}
