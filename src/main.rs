#[macro_use] extern crate log;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate lazy_static;
extern crate serde;
extern crate serde_json;
extern crate url;
extern crate strsim;
extern crate console;
extern crate tungstenite;
extern crate publicsuffix;
extern crate fern;
extern crate chrono;
extern crate idna;

mod util;

use url::Url;
use publicsuffix::List;
use tungstenite::{connect};
use serde_json::{from_str};
use console::{Emoji, style};
use strsim::{damerau_levenshtein};
use idna::punycode::{decode};

static LOOKING_GLASS: Emoji = Emoji("🔍  ", "");
static WEBSOCKET_URL: &'static str = "wss://certstream.calidog.io";

#[derive(Deserialize, Debug)]
struct CertString {
    message_type: String,
    data: Data,
}

#[derive(Deserialize, Debug)]
struct Data {
    leaf_cert: LeafCert,
}

#[derive(Deserialize, Debug)]
struct LeafCert {
    all_domains: Vec<String>,
}

fn main() {
    match setup_logger() {
        Err(why) => panic!("{}", why),
        Ok(_) => (),
    };    

    // Fetch the list from the official URL,
    let list = List::fetch().unwrap();

    let wss: String = format!("{}", WEBSOCKET_URL);
    let url = Url::parse(&wss).unwrap();
    let keywords: Vec<_> = util::KEYWORDS.keys().into_iter().collect();

    match connect(url) {
        Ok(mut answer) => {
            println!("{} {} Fetching Certificates ...", style("[Nettfiske]").bold().dim(), LOOKING_GLASS);

            loop {
                let msg: String = answer.0.read_message().unwrap().into_text().unwrap();
                let cert: CertString = from_str(msg.as_str()).unwrap();

                if cert.message_type.contains("heartbeat") {
                    continue;
                } else if cert.message_type.contains("certificate_update") {
                    for mut domain in cert.data.leaf_cert.all_domains {
 
                        if domain.starts_with("*.") {
                            domain = domain.replace("*.", "");
                        }
                        
                        // Check for any Punycode and rebuild the url
                        // Ie.: xn--80ak6aa92e.com == apple.com
                        let domain_str = punycode(domain.clone());
                        let mut score = domain.matches("xn--").count() * 5;

                        if let Ok(domain) = list.parse_domain(&domain_str) {                           
                            
                            if let Some(registrable) = domain.root() {
                                // Registrable domain
                                let domain_name: Vec<&str> = registrable.split('.').collect();

                                // Subdomain
                                let sub_domain = domain_str.replace(registrable, "");
                                let sub_domain_name: Vec<&str> = sub_domain.split('.').collect();

                                for key in &keywords {
                                    // Check Registration domain
                                    score += domain_keywords(domain_name[0], key, 4);
                                    score += calc_string_edit_distance(domain_name[0], key);
                                    // Check subdomain
                                    score += domain_keywords(sub_domain_name[0], key, 6);
                                    score += calc_string_edit_distance(sub_domain_name[0], key);
                                }

                                // Check for .com, .net on subdomain
                                let tldl: Vec<&str> = vec!["com", "net", "-net", "-com", "net-", "com-"];
                                for key in &tldl {
                                    score += domain_keywords_exact_match(sub_domain_name[0], key, 8);
                                }  
                            }
                        }

                        score += deeply_nested(&domain_str);

                        report(score, &domain_str, &domain);
                    }
                }
            }
        }
        Err(e) => {
            println!("Error during handshake {}", style(e).white());
        }
    }
}

fn deeply_nested(domain: &str) -> usize {
    let v: Vec<&str> = domain.split('.').collect();
    let size = if v.len() >= 3 { v.len() * 3 } else { 0 };
    size
}

fn domain_keywords(name: &str, key: &str, weight: usize) -> usize {
    if name.contains(key) {
        return 10 * weight
    }
    return 0
}

fn domain_keywords_exact_match(name: &str, key: &str, weight: usize) -> usize {
    if name.eq_ignore_ascii_case(key) {
        return 10 * weight
    }
    return 0
}

// Damerau Levenshtein: Calculates number of operations (Insertions, deletions or substitutions,
// or transposition of two adjacent characters) required to change one word into the other.
fn calc_string_edit_distance(name: &str, key: &str) -> usize {
    if damerau_levenshtein(name, key) == 1 {
        return 40
    }
    return 0
}

// Decode the domain as Punycode
fn punycode(domain: String) -> String {
    let mut result = Vec::new();
    let words_list: Vec<&str> = domain.split('.').collect();

    for word in &words_list {
        if word.starts_with("xn--") {
            let pu = word.replace("xn--", "");
            let decoded = decode(&pu).unwrap().into_iter().collect::<String>();
            result.push(decoded);
        } else {
            result.push(word.to_string());
        }
    }

    result.join(".")
}

fn report(score: usize, domain: &str, domain_original: &str) {
    if score >= 76 {
        print_domain(score, style(domain).red(), domain_original);
    } else if score >= 68 {
        print_domain(score, style(domain).magenta(), domain_original);
    } else if score >= 56 {
        print_domain(score, style(domain).yellow(), domain_original);
    }

    if score >= 56 {
        if domain_original.matches("xn--").count() > 0 {
            info!("{} - (Punycode: {})", domain, domain_original);
        } else {
            info!("{}", domain); 
        }
    }
}

fn print_domain(score: usize, styled_domain: console::StyledObject<&str>, domain_original: &str) {
    if domain_original.matches("xn--").count() > 0 {
        println!("Suspicious {} (score {}) (Punycode: {})", styled_domain, score, domain_original);     
    } else {
        println!("Suspicious {} (score {})", styled_domain, score);      
    }    
}

fn setup_logger() -> Result<(), fern::InitError> {
    fern::Dispatch::new()
        .format(|out, message, _record| {
            out.finish(format_args!(
                "{} {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                message
            ))
        })
        .level(log::LevelFilter::Info)
        .chain(fern::log_file("nettfiske.log")?)
        .apply()?;
    Ok(())
}