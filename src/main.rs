use chrono::{Datelike, Local, NaiveDate};
use dotenvy::dotenv;
use headless_chrome::Tab;

use headless_chrome::protocol::cdp::Page;

use headless_chrome::{Browser, LaunchOptionsBuilder};
use std::env;
use std::fs;
use std::io::{self, Read};
use std::{
    collections::HashSet,
    error::Error,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();

    let today: NaiveDate = Local::now().date_naive();

    let (first_day, last_day) = first_and_last_day_of_month(today);

    let username_value = env::var("USERNAME").expect("USERNAME not set");
    let password_value = env::var("PASSWORD").expect("PASSWORD not set");

    let launch_options = LaunchOptionsBuilder::default().headless(false).build()?;

    let browser = Browser::new(launch_options)?;

    let tab = browser.new_tab()?;
    let cwd: PathBuf = env::current_dir()?;
    enable_downloads_to_dir(&tab, &cwd)?; // allow downloads into current folder

    tab.navigate_to("https://sarel.mangotopsrv.com/reports/search/")?;
    tab.wait_until_navigated()?;

    // login
    let username_input = tab.find_element("input#id_username")?;

    username_input.click()?;

    username_input
        .type_into(&username_value)
        .map_err(|e| format!("Failed to type username: {}", e))?;

    let password_input = tab.find_element("input#id_password")?;

    password_input.click()?;

    password_input
        .type_into(&password_value)
        .map_err(|e| format!("Failed to type password: {}", e))?;

    let submit_button = tab.find_element("button#btn")?;

    submit_button.click()?;

    println!("logged in...");

    // file search has to load
    let from_date = tab.wait_for_element("input[name='from_date']")?;

    from_date.click()?;

    from_date
        .type_into(&date_formatter(first_day))
        .map_err(|e| format!("Failed to type from date: {}", e))?;

    let to_date = tab.find_element("input[name=to_date]")?;

    to_date.click()?;

    to_date
        .type_into(&date_formatter(last_day))
        .map_err(|e| format!("Failed to type to date: {}", e))?;

    let search_submit = tab.find_element("button[type=submit]")?;

    search_submit.click()?;

    println!("searching dates...");

    let before = snapshot_files(&cwd)?;

    tab.wait_for_element("button[class=table-csv-export-btn]")?
        .click()?;

    let downloaded = wait_for_new_download(&cwd, &before, Duration::from_secs(60))?;
    println!("Downloaded file: {}", downloaded.display());

    println!("Chrome is open. Press ENTER to close...");
    let _ = io::stdin().read(&mut [0u8])?;

    Ok(())
}

fn enable_downloads_to_dir(tab: &Tab, dir: &Path) -> Result<(), Box<dyn Error>> {
    // Chrome wants an absolute path for downloads
    let abs = dir.canonicalize()?;

    tab.call_method(Page::SetDownloadBehavior {
        behavior: Page::SetDownloadBehaviorBehaviorOption::Allow,
        download_path: Some(abs.to_string_lossy().to_string()),
    })?;

    Ok(())
}

fn snapshot_files(dir: &Path) -> Result<HashSet<String>, Box<dyn Error>> {
    let mut set = HashSet::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str() {
            set.insert(name.to_string());
        }
    }
    Ok(set)
}

fn wait_for_new_download(
    dir: &Path,
    before: &HashSet<String>,
    timeout: Duration,
) -> Result<PathBuf, Box<dyn Error>> {
    let start = Instant::now();

    while start.elapsed() < timeout {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();

            // ignore files that were already there
            if before.contains(&name) {
                continue;
            }

            // ignore partial Chrome downloads
            if name.ends_with(".crdownload") {
                continue;
            }

            return Ok(entry.path());
        }

        std::thread::sleep(Duration::from_millis(250));
    }

    Err(format!("Download timed out after {:?}", timeout).into())
}

fn date_formatter(date: NaiveDate) -> String {
    let dd = format!("{:02}", date.day());
    let mm = format!("{:02}", date.month());
    let yy = date.year() % 100;

    format!("{}/{}/{:02}", dd, mm, yy)
}

fn first_and_last_day_of_month(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first_day = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();

    let (next_year, next_month) = if today.month() == 12 {
        (today.year() + 1, 1)
    } else {
        (today.year(), today.month() + 1)
    };

    let first_of_next_month = NaiveDate::from_ymd_opt(next_year, next_month, 1).unwrap();

    let last_day = first_of_next_month.pred_opt().unwrap();

    (first_day, last_day)
}
