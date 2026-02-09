use chrono::{Datelike, Local, NaiveDate};
use csv::ReaderBuilder;
use delete::delete_file;
use dotenvy::{dotenv /* from_path */};
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;
use headless_chrome::{Browser, LaunchOptionsBuilder};
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::env;
// use std::fmt::format;
use std::fs;
use std::{
    collections::HashSet,
    error::Error,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use strsim::jaro_winkler;

#[derive(Deserialize, Debug)]
struct Row {
    // id: i32,
    // created_at: String,
    converted_lead_email: String,
    account_name: String,
}

#[derive(Hash, Eq, PartialEq, Debug, Serialize)]
struct ContactHookKey {
    email: String,
    file_number: String,
}

impl ContactHookKey {
    fn new(email: &str, file_number: &str) -> Self {
        Self {
            email: email.trim().to_lowercase(),
            file_number: file_number.to_string(),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();

    top_table_download().await?;

    // Getting the supabase table records
    let supabase_rows = supabase_table_read().await?;

    // Reading the csv file records

    let records: Vec<HashMap<String, String>> = read_csv("table.csv", &supabase_rows).await?;

    zoho_webhook(records, &supabase_rows).await?;

    delete_file("table.csv").unwrap();

    Ok(())
}

async fn zoho_webhook(
    records: Vec<HashMap<String, String>>,
    supabase_rows: &[Row],
) -> Result<(), Box<dyn Error>> {
    let client = Client::new();
    let webhook_url = env::var("ZOHO_WEBHOOK").expect("ZOHO_WEBHOOK not set");
    let mut files = Vec::new();
    let mut seen_emails = HashSet::new();

    for record in &records {
        if let (Some(agent_val), Some(file_name)) = (record.get("Agent"), record.get("File")) {
            let cleaned_agent = agent_val.trim().to_lowercase();

            let best_match = supabase_rows.iter().find(|row| {
                let stored_clean = row.account_name.trim().to_lowercase();
                strsim::jaro_winkler(&cleaned_agent, &stored_clean) >= 0.9
            });

            if let Some(matched_row) = best_match
                && !seen_emails.contains(&matched_row.converted_lead_email)
            {
                println!(
                    "Unique Match Found: {} -> {} | file -> {}",
                    matched_row.converted_lead_email, agent_val, file_name
                );

                if let Some(number) = extract_file_number(file_name) {
                    println!("Extracted number: {}", number);

                    let file_link = format!("https://sarel.mangotopsrv.com/file/{}", &number);

                    let contact_hook =
                        ContactHookKey::new(&matched_row.converted_lead_email, &file_link);

                    let response = client
                        .post(&webhook_url) // Add the & here to borrow it instead of moving it
                        .json(&contact_hook)
                        .send()
                        .await?;

                    println!("Contact Hook Created: {:?}", contact_hook);
                } else {
                    println!("Could not find a #number in the file string: {}", file_name);
                }

                seen_emails.insert(matched_row.converted_lead_email.clone());
                files.push(matched_row);
            }
        }
    }

    println!("Total unique leads found: {:?}", files.len());

    Ok(())
}

// only valid use of AI :)
fn extract_file_number(text: &str) -> Option<String> {
    // Regex breakdown:
    // #      - looks for the pound sign
    // (\d+)  - captures one or more digits into group 1
    // \s?    - looks for a space (optional)
    let re = Regex::new(r"#(\d+)\s?").unwrap();

    re.captures(text)
        .and_then(|cap| cap.get(1)) // Get the first capture group
        .map(|m| m.as_str().to_string()) // Convert to String
}

async fn read_csv(
    path: &str,
    supabase_rows: &[Row],
) -> Result<Vec<HashMap<String, String>>, Box<dyn Error>> {
    let mut reader = ReaderBuilder::new()
        .delimiter(b';')
        .has_headers(true)
        .from_path(path)?;

    // results from csv file
    let mut results = Vec::new();

    for result in reader.deserialize() {
        let record: HashMap<String, String> = result?;
        if let Some(agent) = record.get("Agent") {
            let cleaned_agent = agent.trim().to_lowercase();

            // string lookup threshold
            let threshold = 0.9;

            let best_match = supabase_rows.iter().find(|row| {
                let stored_clean = row.account_name.trim().to_lowercase();
                jaro_winkler(&cleaned_agent, &stored_clean) >= threshold
            });

            if let Some(row) = best_match {
                results.push(record);
            }
        }
    }

    Ok(results)
}

async fn top_table_download() -> Result<(), Box<dyn Error>> {
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

    Ok(())
}

fn enable_downloads_to_dir(tab: &Tab, dir: &Path) -> Result<(), Box<dyn Error>> {
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

            if before.contains(&name) {
                continue;
            }

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

async fn supabase_table_read() -> Result<Vec<Row>, Box<dyn std::error::Error>> {
    let sup_project_id = env::var("SUPABASE_URL").expect("SUPABASE_URL not set");

    let supabase_url = format!("https://{}.supabase.co", sup_project_id);
    let anon_key = env::var("SUPABASE_ANON_KEY").expect("SUPABASE_ANON_KEY not set");

    let url = format!(
        "{}/rest/v1/converted_leads?select=id,converted_lead_email,created_at,account_name&limit=5",
        supabase_url
    );

    let res = Client::new()
        .get(&url)
        .header("apikey", &anon_key)
        .header("Authorization", format!("Bearer {}", &anon_key))
        .send()
        .await?;

    let status = res.status();
    let body = res.text().await?;

    // parseing if request succesful
    if !status.is_success() {
        return Err(format!("Request failed: {}", status).into());
    }

    let rows: Vec<Row> = serde_json::from_str(&body)?;

    Ok(rows)
}
