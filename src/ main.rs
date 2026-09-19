// buzz - basically yay but for debian, named after the first debian release
// (1.1 was called "Buzz" apparently, thought that was a fun name to steal)
//
// usage is like:
//   buzz firefox          -> searches + gives you a numbered list to pick from
//   buzz install <thing>  -> tries apt first (fast), only builds from source
//                            if there's no binary or you pass --build
//   buzz search <thing>
//   buzz upgrade
//   buzz remove <thing>
//   buzz clean            -> nukes the build cache
//
// anything i didn't handle just gets forwarded to apt-get so it doesn't
// break normal apt usage
//
// no external crates on purpose, didn't want to deal with cargo pulling in
// half of crates.io for something this small. wrote my own tiny json thing
// below for the state file, it's not "real" json parsing but it does what
// i need
 
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
 
const APT_ONLY_VERBS: &[&str] = &[
    // note: "list" used to be here but buzz has its own list command now,
    // use `buzz apt-list` style passthrough via apt directly if you want
    // the raw apt version
    "update", "autoremove", "purge", "show", "depends", "rdepends",
    "policy", "dist-upgrade", "source", "download", "changelog", "autoclean",
];
 
// -- paths --
 
fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/root"))
}
fn buzz_home() -> PathBuf { home_dir().join(".cache").join("buzz") }
fn default_build_dir() -> PathBuf { buzz_home().join("build") }
fn cache_dir() -> PathBuf { buzz_home().join("cache") }
fn state_file() -> PathBuf { buzz_home().join("state.json") }
 
// -- color / logging --
 
fn use_color() -> bool { io::stdout().is_terminal() }
 
fn c(code: &str, text: &str) -> String {
    if use_color() { format!("\x1b[{}m{}\x1b[0m", code, text) } else { text.to_string() }
}
fn green(t: &str) -> String { c("32;1", t) }
fn cyan(t: &str) -> String { c("36;1", t) }
fn red(t: &str) -> String { c("31;1", t) }
fn yellow(t: &str) -> String { c("33;1", t) }
fn bold(t: &str) -> String { c("1", t) }
fn dim(t: &str) -> String { c("2", t) }
fn magenta(t: &str) -> String { c("35;1", t) } // used for flatpak results in the picker
 
fn log(msg: &str, level: char) {
    // using chars instead of strings here bc i didnt wanna type out
    // "success"/"error"/"info" everywhere, chars are quicker
    let marker = match level {
        '+' => green("[+]"),
        '-' => red("[-]"),
        '*' => cyan("[*]"),
        other => format!("[{}]", other), // shouldnt really hit this but whatever
    };
    println!("{} {}", marker, msg);
}
 
// prints a big obvious "this actually finished" banner. used after
// anything that changes the system (install/remove/upgrade/clean) so
// someone new to the terminal isn't left staring at silence wondering
// if it worked or if it's still doing something
fn finish(msg: &str) {
    let border_len = msg.chars().count().clamp(24, 60);
    let border = "=".repeat(border_len);
    println!();
    println!("{}", green(&border));
    println!("{}", bold(&green(&format!("✔ {}", msg))));
    println!("{}", green(&border));
    println!();
}
 
// same idea as finish() but for the failure case - big obvious red
// banner so someone new to all this doesn't have to go hunting through
// a wall of apt output to figure out that something actually broke
fn fail_banner(msg: &str) {
    let border_len = msg.chars().count().clamp(24, 60);
    let border = "=".repeat(border_len);
    println!();
    println!("{}", red(&border));
    println!("{}", bold(&red(&format!("✘ Something went wrong"))));
    println!("{}", red(&border));
    println!();
}
 
fn die(msg: &str) -> ! {
    fail_banner(msg);
    log(msg, '-');
    // exit(1) not exit(0) obviously since this is the error path
    std::process::exit(1);
}
 
fn confirm(prompt: &str, default_yes: bool, noconfirm: bool) -> bool {
    if noconfirm {
        return default_yes;
    }
    let suffix = if default_yes { " [Y/n] " } else { " [y/N] " };
    print!("{}{}{}", bold("==> "), prompt, suffix);
    io::stdout().flush().ok(); // gotta flush or the prompt doesnt show before read_line blocks
    let mut ans = String::new();
    io::stdin().read_line(&mut ans).ok();
    let ans = ans.trim().to_lowercase();
    if ans.is_empty() {
        return default_yes;
    }
    // yes/y both work, figured people might type either
    ans == "y" || ans == "yes"
}
 
// -- process execution --
 
fn run_capture(cmd: &[&str], cwd: Option<&Path>) -> String {
    let mut command = Command::new(cmd[0]);
    command.args(&cmd[1..]);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    match command.output() {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).to_string(),
        Ok(output) => die(&format!(
            "Command failed ({}): {}\n{}",
            output.status,
            cmd.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )),
        Err(e) => die(&format!("Command not found: {} ({})", cmd[0], e)),
    }
}
 
fn running_as_root() -> bool {
    run_capture(&["id", "-u"], None).trim() == "0"
}
 
fn run(cmd: &[&str], cwd: Option<&Path>, sudo: bool) {
    let mut full: Vec<String> = Vec::new();
    if sudo && !running_as_root() {
        full.push("sudo".to_string());
    }
    full.extend(cmd.iter().map(|s| s.to_string()));
    log(&dim(&format!("Running: {}", full.join(" "))), '*');
 
    let mut command = Command::new(&full[0]);
    command.args(&full[1..]);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    match command.status() {
        Ok(status) if status.success() => {}
        Ok(status) => die(&format!(
            "Command failed ({}): {}",
            status.code().unwrap_or(-1),
            full.join(" ")
        )),
        Err(e) => die(&format!("Command not found: {} ({})", full[0], e)),
    }
}
 
// same as run() but retries a few times before giving up - only meant for
// the commands that actually talk to the network (apt-get update/source,
// git clone, flatpak install), since retrying something like a bad
// dependency install would just waste time failing the same way 3x
fn run_retry(cmd: &[&str], cwd: Option<&Path>, sudo: bool, attempts: u32) {
    let mut full: Vec<String> = Vec::new();
    if sudo && !running_as_root() {
        full.push("sudo".to_string());
    }
    full.extend(cmd.iter().map(|s| s.to_string()));
 
    for attempt in 1..=attempts {
        if attempt == 1 {
            log(&dim(&format!("Running: {}", full.join(" "))), '*');
        } else {
            log(&format!("Retrying ({}/{}): {}", attempt, attempts, full.join(" ")), '*');
        }
 
        let mut command = Command::new(&full[0]);
        command.args(&full[1..]);
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
 
        match command.status() {
            Ok(status) if status.success() => return,
            Ok(_) if attempt < attempts => {
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
            Ok(status) => die(&format!(
                "Command failed after {} attempt(s) ({}): {}",
                attempts,
                status.code().unwrap_or(-1),
                full.join(" ")
            )),
            Err(e) if attempt < attempts => {
                // command not found isn't going to fix itself by waiting, but
                // this branch stays generic so a transient spawn failure
                // (rare, but possible under load) still gets another shot
                log(&format!("Spawn failed ({}), retrying ...", e), '*');
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
            Err(e) => die(&format!("Command not found: {} ({})", full[0], e)),
        }
    }
}
 
fn is_git_url(target: &str) -> bool {
    target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("git@")
        || target.ends_with(".git")
}
 
fn fresh_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).ok();
    }
    fs::create_dir_all(path).ok();
}
 
fn now_iso() -> String {
    run_capture(&["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"], None).trim().to_string()
}
 
// tiny homemade json stuff below, only handles what i actually need
// (strings, arrays, objects, null) - no numbers even bc i never store any.
// probably has edge cases it doesnt handle but it works for my state file
 
#[derive(Debug, Clone)]
enum Json {
    Null,
    Bool(bool),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}
 
impl Json {
    fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self { Some(s.as_str()) } else { None }
    }
    fn as_arr(&self) -> Option<&Vec<Json>> {
        if let Json::Arr(a) = self { Some(a) } else { None }
    }
    fn as_obj(&self) -> Option<&Vec<(String, Json)>> {
        if let Json::Obj(o) = self { Some(o) } else { None }
    }
    fn get(&self, key: &str) -> Option<&Json> {
        self.as_obj()?.iter().find(|(k, _)| k.as_str() == key).map(|(_, v)| v)
    }
}
 
fn json_escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
 
fn json_to_string(v: &Json, indent: usize) -> String {
    match v {
        Json::Null => "null".to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Str(s) => format!("\"{}\"", json_escape(s)),
        Json::Arr(items) => {
            if items.is_empty() {
                return "[]".to_string();
            }
            let pad = "  ".repeat(indent + 1);
            let closing = "  ".repeat(indent);
            let body: Vec<String> =
                items.iter().map(|it| format!("{}{}", pad, json_to_string(it, indent + 1))).collect();
            format!("[\n{}\n{}]", body.join(",\n"), closing)
        }
        Json::Obj(pairs) => {
            if pairs.is_empty() {
                return "{}".to_string();
            }
            let pad = "  ".repeat(indent + 1);
            let closing = "  ".repeat(indent);
            let body: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}\"{}\": {}", pad, json_escape(k), json_to_string(v, indent + 1)))
                .collect();
            format!("{{\n{}\n{}}}", body.join(",\n"), closing)
        }
    }
}
 
fn parse_json(text: &str) -> Json {
    let chars: Vec<char> = text.chars().collect();
    let mut pos = 0usize;
    parse_value(&chars, &mut pos)
}
 
fn skip_ws(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_whitespace() {
        *pos += 1;
    }
}
 
fn parse_value(chars: &[char], pos: &mut usize) -> Json {
    skip_ws(chars, pos);
    match chars.get(*pos) {
        Some('{') => parse_object(chars, pos),
        Some('[') => parse_array(chars, pos),
        Some('"') => Json::Str(parse_string(chars, pos)),
        Some('t') => { *pos += 4; Json::Bool(true) }
        Some('f') => { *pos += 5; Json::Bool(false) }
        Some('n') => { *pos += 4; Json::Null }
        _ => Json::Null,
    }
}
 
fn parse_string(chars: &[char], pos: &mut usize) -> String {
    *pos += 1; // opening quote
    let mut s = String::new();
    while let Some(&ch) = chars.get(*pos) {
        if ch == '"' {
            *pos += 1;
            break;
        }
        if ch == '\\' {
            *pos += 1;
            match chars.get(*pos) {
                Some('n') => s.push('\n'),
                Some('t') => s.push('\t'),
                Some('"') => s.push('"'),
                Some('\\') => s.push('\\'),
                Some(other) => s.push(*other),
                None => {}
            }
            *pos += 1;
        } else {
            s.push(ch);
            *pos += 1;
        }
    }
    s
}
 
fn parse_object(chars: &[char], pos: &mut usize) -> Json {
    *pos += 1; // {
    let mut pairs = Vec::new();
    loop {
        skip_ws(chars, pos);
        if chars.get(*pos) == Some(&'}') {
            *pos += 1;
            break;
        }
        let key = parse_string(chars, pos);
        skip_ws(chars, pos);
        if chars.get(*pos) == Some(&':') {
            *pos += 1;
        }
        let val = parse_value(chars, pos);
        pairs.push((key, val));
        skip_ws(chars, pos);
        if chars.get(*pos) == Some(&',') {
            *pos += 1;
            continue;
        }
        skip_ws(chars, pos);
        if chars.get(*pos) == Some(&'}') {
            *pos += 1;
        }
        break;
    }
    Json::Obj(pairs)
}
 
fn parse_array(chars: &[char], pos: &mut usize) -> Json {
    *pos += 1; // [
    let mut items = Vec::new();
    loop {
        skip_ws(chars, pos);
        if chars.get(*pos) == Some(&']') {
            *pos += 1;
            break;
        }
        let val = parse_value(chars, pos);
        items.push(val);
        skip_ws(chars, pos);
        if chars.get(*pos) == Some(&',') {
            *pos += 1;
            continue;
        }
        skip_ws(chars, pos);
        if chars.get(*pos) == Some(&']') {
            *pos += 1;
        }
        break;
    }
    Json::Arr(items)
}
 
// -- state: tracks packages buzz has built from source --
 
#[derive(Clone)]
struct PkgEntry {
    source_kind: String,
    source: String,
    version: Option<String>,
    pkg_dir: String,
    built_debs: Vec<String>,
    installed_at: String,
}
 
type State = Vec<(String, PkgEntry)>;
 
fn load_state() -> State {
    let path = state_file();
    if !path.exists() {
        return Vec::new();
    }
    let text = fs::read_to_string(&path).unwrap_or_default();
    let json = parse_json(&text);
    let mut result = Vec::new();
    if let Some(pkgs) = json.get("packages").and_then(|v| v.as_obj()) {
        for (name, entry) in pkgs {
            let source_kind = entry.get("source_kind").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let source = entry.get("source").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let version = entry.get("version").and_then(|v| v.as_str()).map(|s| s.to_string());
            let pkg_dir = entry.get("pkg_dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let built_debs = entry
                .get("built_debs")
                .and_then(|v| v.as_arr())
                .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default();
            let installed_at = entry.get("installed_at").and_then(|v| v.as_str()).unwrap_or("").to_string();
            result.push((
                name.clone(),
                PkgEntry { source_kind, source, version, pkg_dir, built_debs, installed_at },
            ));
        }
    }
    result
}
 
fn save_state(state: &State) {
    let mut pkg_pairs = Vec::new();
    for (name, entry) in state {
        let obj = vec![
            ("source_kind".to_string(), Json::Str(entry.source_kind.clone())),
            ("source".to_string(), Json::Str(entry.source.clone())),
            (
                "version".to_string(),
                match &entry.version {
                    Some(v) => Json::Str(v.clone()),
                    None => Json::Null,
                },
            ),
            ("pkg_dir".to_string(), Json::Str(entry.pkg_dir.clone())),
            (
                "built_debs".to_string(),
                Json::Arr(entry.built_debs.iter().map(|d| Json::Str(d.clone())).collect()),
            ),
            ("installed_at".to_string(), Json::Str(entry.installed_at.clone())),
        ];
        pkg_pairs.push((name.clone(), Json::Obj(obj)));
    }
    let root = Json::Obj(vec![("packages".to_string(), Json::Obj(pkg_pairs))]);
    let text = json_to_string(&root, 0);
    fs::create_dir_all(buzz_home()).ok();
    fs::write(state_file(), text).ok();
}
 
fn state_find<'a>(state: &'a State, name: &str) -> Option<&'a PkgEntry> {
    state.iter().find(|(n, _)| n == name).map(|(_, e)| e)
}
fn state_upsert(state: &mut State, name: &str, entry: PkgEntry) {
    if let Some(pos) = state.iter().position(|(n, _)| n == name) {
        state[pos] = (name.to_string(), entry);
    } else {
        state.push((name.to_string(), entry));
    }
}
fn state_remove(state: &mut State, name: &str) {
    state.retain(|(n, _)| n != name);
}
 
fn get_pkg_version(pkg_dir: &Path) -> Option<String> {
    let changelog = pkg_dir.join("debian").join("changelog");
    if !changelog.exists() {
        return None;
    }
    let out = run_capture(&["dpkg-parsechangelog", "-S", "Version"], Some(pkg_dir));
    Some(out.trim().to_string())
}
 
// -- source fetching / building --
 
fn clone_source(repo_url: &str, dest: &Path) -> PathBuf {
    if let Some(parent) = dest.parent() {
        fresh_dir(parent);
    }
    log(&format!("Cloning {} ...", repo_url), '+');
    let dest_str = dest.to_string_lossy().to_string();
    run_retry(&["git", "clone", "--depth", "1", repo_url, &dest_str], None, false, 3);
    dest.to_path_buf()
}
 
// checks whether deb-src lines are actually enabled anywhere. apt-get
// source is useless without them and the error message apt gives is
// pretty opaque if you've never seen it before
fn deb_src_enabled() -> bool {
    // apt-get indextargets is the "proper" way to ask apt what it knows
    // about, rather than parsing config files ourselves
    match Command::new("apt-get").args(["indextargets", "--no-release-info"]).output() {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.contains("deb-src") || text.to_lowercase().contains("sources")
        }
        // if that fails for any reason fall back to grepping the config
        // files directly - crude but works on both formats
        _ => {
            let classic = fs::read_to_string("/etc/apt/sources.list").unwrap_or_default();
            if classic.lines().any(|l| l.trim_start().starts_with("deb-src")) {
                return true;
            }
            if let Ok(entries) = fs::read_dir("/etc/apt/sources.list.d") {
                for e in entries.filter_map(|e| e.ok()) {
                    let content = fs::read_to_string(e.path()).unwrap_or_default();
                    let has_classic = content.lines().any(|l| l.trim_start().starts_with("deb-src"));
                    let has_deb822 = content.lines().any(|l| {
                        let lt = l.trim_start();
                        lt.starts_with("Types:") && lt.contains("deb-src")
                    });
                    if has_classic || has_deb822 {
                        return true;
                    }
                }
            }
            false
        }
    }
}
 
// offers to turn deb-src on rather than making the person go hunt down
// sed commands. handles both the classic sources.list format and the
// newer deb822 .sources format (ubuntu 24.04+, and derivatives)
fn offer_enable_deb_src(noconfirm: bool) -> bool {
    log("Source packages (deb-src) aren't enabled in your apt sources.", '-');
    log("Without them, apt can't download source code to build from.", '*');
 
    if !confirm("Enable deb-src automatically now?", true, noconfirm) {
        log("Left apt sources unchanged.", '-');
        return false;
    }
 
    let mut changed = false;
 
    // deb822 format first (.sources files) - flip "Types: deb" into
    // "Types: deb deb-src"
    if let Ok(entries) = fs::read_dir("/etc/apt/sources.list.d") {
        for e in entries.filter_map(|e| e.ok()) {
            let path = e.path();
            if path.extension().map(|x| x == "sources").unwrap_or(false) {
                let content = fs::read_to_string(&path).unwrap_or_default();
                if content.lines().any(|l| l.trim() == "Types: deb") {
                    let patched: String = content
                        .lines()
                        .map(|l| if l.trim() == "Types: deb" { "Types: deb deb-src" } else { l })
                        .collect::<Vec<&str>>()
                        .join("\n");
                    let tmp = std::env::temp_dir().join("buzz-sources-patch");
                    if fs::write(&tmp, format!("{}\n", patched)).is_ok() {
                        run(&["cp", &tmp.to_string_lossy(), &path.to_string_lossy()], None, true);
                        log(&format!("Enabled deb-src in {}", path.display()), '+');
                        changed = true;
                    }
                }
            }
        }
    }
 
    // classic format - duplicate each "deb " line as a "deb-src " line
    let classic_path = "/etc/apt/sources.list";
    let classic = fs::read_to_string(classic_path).unwrap_or_default();
    let already_has_src = classic.lines().any(|l| l.trim_start().starts_with("deb-src"));
    if !classic.trim().is_empty() && !already_has_src {
        let mut out_lines: Vec<String> = Vec::new();
        for line in classic.lines() {
            out_lines.push(line.to_string());
            let lt = line.trim_start();
            if lt.starts_with("deb ") {
                out_lines.push(lt.replacen("deb ", "deb-src ", 1));
            }
        }
        let tmp = std::env::temp_dir().join("buzz-sourceslist-patch");
        if fs::write(&tmp, format!("{}\n", out_lines.join("\n"))).is_ok() {
            run(&["cp", &tmp.to_string_lossy(), classic_path], None, true);
            log(&format!("Enabled deb-src in {}", classic_path), '+');
            changed = true;
        }
    }
 
    if !changed {
        log("Couldn't find anything to change in your apt sources.", '-');
        return false;
    }
 
    log("Refreshing package lists ...", '+');
    run_retry(&["apt-get", "update"], None, true, 3);
    true
}
 
fn fetch_debian_source(pkg_name: &str, build_root: &Path) -> PathBuf {
    // check up front instead of letting apt fail with its cryptic
    // "You must put some 'deb-src' URIs in your sources.list" message
    if !deb_src_enabled() {
        offer_enable_deb_src(false);
    }
 
    fresh_dir(build_root);
    log(&format!("Fetching Debian source package '{}' via apt-get source ...", pkg_name), '+');
    run_retry(&["apt-get", "source", pkg_name], Some(build_root), false, 3);
    if let Ok(entries) = fs::read_dir(build_root) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().is_dir() {
                return entry.path();
            }
        }
    }
    die("apt-get source produced no directory. Make sure deb-src lines are enabled in your apt sources.")
}
 
fn install_build_dependencies(pkg_dir: &Path) {
    log("Installing build dependencies ...", '+');
    if pkg_dir.join("debian").join("control").exists() {
        run(&["apt-get", "build-dep", "-y", "."], Some(pkg_dir), true);
    } else {
        log("No debian/control found — installing generic build toolchain.", '+');
        run(&["apt-get", "install", "-y", "build-essential", "debhelper", "devscripts"], None, true);
    }
}
 
fn build_deb_package(pkg_dir: &Path) {
    log("Building .deb package (unsigned, local build) ...", '+');
    run(&["dpkg-buildpackage", "-us", "-uc", "-b"], Some(pkg_dir), false);
}
 
fn collect_built_debs(pkg_dir: &Path) -> Vec<PathBuf> {
    let parent = pkg_dir.parent().unwrap_or(pkg_dir);
    let mut debs: Vec<PathBuf> = fs::read_dir(parent)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "deb").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    debs.sort();
    if debs.is_empty() {
        die("No .deb files were produced by the build.");
    }
    debs
}
 
fn cache_debs(name: &str, version: &Option<String>, debs: &[PathBuf]) -> Vec<PathBuf> {
    let verstr = version.clone().unwrap_or_else(|| "unversioned".to_string());
    let dest_dir = cache_dir().join(format!("{}-{}", name, verstr));
    fs::create_dir_all(&dest_dir).ok();
    let mut cached = Vec::new();
    for deb in debs {
        if let Some(fname) = deb.file_name() {
            let target = dest_dir.join(fname);
            fs::copy(deb, &target).ok();
            cached.push(target);
        }
    }
    cached
}
 
fn install_debs(debs: &[PathBuf]) {
    let names: Vec<String> =
        debs.iter().map(|d| d.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()).collect();
    log(&format!("Installing: {}", names.join(", ")), '+');
    let path_strs: Vec<String> = debs.iter().map(|d| d.to_string_lossy().to_string()).collect();
    let mut cmd: Vec<&str> = vec!["apt-get", "install", "-y"];
    let path_refs: Vec<&str> = path_strs.iter().map(|s| s.as_str()).collect();
    cmd.extend(path_refs);
    run(&cmd, None, true);
}
 
// -- core install flow --
 
// checks if apt already has a built version of this so we don't have to
// compile everything from scratch every time lol
fn package_binary_available(name: &str) -> bool {
    let out = Command::new("apt-cache").args(["policy", name]).output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            // apt-cache policy prints "Candidate: (none)" when nothing exists,
            // otherwise it prints the actual candidate version
            text.contains("Candidate:") && !text.contains("Candidate: (none)")
        }
        _ => false, // if the check fails just assume no, fall back to source build
    }
}
 
// same idea as package_binary_available but for flatpak. flatpak might not
// even be installed on the system so gotta check that first
fn flatpak_available() -> bool {
    Command::new("flatpak").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}
 
// flathub is the actual app store basically, flatpak on its own doesn't
// have any apps configured by default on most systems
fn flatpak_has_flathub() -> bool {
    match Command::new("flatpak").args(["remote-list"]).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).contains("flathub"),
        _ => false,
    }
}
 
fn flatpak_ensure_flathub(noconfirm: bool) {
    if !flatpak_available() || flatpak_has_flathub() {
        return;
    }
    if confirm("Flatpak is installed but the flathub remote isn't set up. Add it now?", true, noconfirm) {
        run_retry(
            &["flatpak", "remote-add", "--if-not-exists", "flathub", "https://flathub.org/repo/flathub.flatpakrepo"],
            None,
            true,
            3,
        );
    }
}
 
// returns (app_id, name, description) for each match. flatpak search prints
// tab separated columns, no header row on most versions i've seen
fn flatpak_search_raw(term: &str) -> Vec<(String, String, String)> {
    if !flatpak_available() {
        return Vec::new();
    }
    let out = Command::new("flatpak").args(["search", term]).output();
    let text = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return Vec::new(),
    };
    let mut results = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        // Name, Description, Application ID, Version, Branch, Remotes - in that order
        if cols.len() >= 3 {
            let name = cols[0].trim().to_string();
            let desc = cols[1].trim().to_string();
            let app_id = cols[2].trim().to_string();
            if !app_id.is_empty() {
                results.push((app_id, name, desc));
            }
        }
    }
    results
}
 
// fuzzy lookup on flathub for something like "obs studios" or "obs-studio"
// that should still find "OBS Studio" / com.obsproject.Studio. flatpak's
// own search sometimes misses hyphenated queries, so if the raw term comes
// back empty we retry with hyphens/underscores turned into spaces before
// giving up
fn find_flatpak_search_matches(term: &str) -> Vec<(String, String)> {
    let norm_term = normalize(term);
    let variants = [term.to_string(), term.replace(['-', '_'], " ")];
 
    for variant in variants.iter() {
        let hits: Vec<(String, String)> = flatpak_search_raw(variant)
            .into_iter()
            .filter(|(app_id, name, _)| {
                let id_norm = normalize(app_id);
                let name_norm = normalize(name);
                id_norm.contains(&norm_term) || norm_term.contains(&id_norm) || name_norm.contains(&norm_term) || norm_term.contains(&name_norm)
            })
            .map(|(app_id, name, desc)| (app_id, format!("{} - {}", name, desc)))
            .collect();
        if !hits.is_empty() {
            return hits;
        }
    }
    Vec::new()
}
 
// same narrowing idea as find_apt_search_matches, but for the interactive
// picker's flatpak column - keeps the full (app_id, name, desc) shape so
// the picker can still show a readable label, not just the app id.
// falls back to the raw list if narrowing empties it out entirely
fn find_flatpak_matches_full(term: &str) -> Vec<(String, String, String)> {
    let norm_term = normalize(term);
    let all = flatpak_search_raw(term);
    let narrowed: Vec<(String, String, String)> = all
        .iter()
        .filter(|(app_id, name, _)| {
            let id_norm = normalize(app_id);
            let name_norm = normalize(name);
            id_norm.contains(&norm_term) || norm_term.contains(&id_norm) || name_norm.contains(&norm_term) || norm_term.contains(&name_norm)
        })
        .cloned()
        .collect();
    if narrowed.is_empty() {
        all
    } else {
        narrowed
    }
}
 
// checks dpkg's own database instead of apt-cache, since this is about
// whats ACTUALLY installed right now, not what could be installed
fn is_apt_installed(name: &str) -> bool {
    Command::new("dpkg").args(["-s", name]).output().map(|o| o.status.success()).unwrap_or(false)
}
 
// looks through currently-installed flatpak apps (not flathub search
// results) for something matching by app id or display name. returns the
// app id if found since thats what flatpak uninstall actually wants
// strips out spaces/dashes/underscores/case so "obs-studio", "obs studio",
// "OBS Studio" and "obsstudio" all compare equal. this is what makes
// vague/misremembered names still match
fn normalize(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}
 
// when more than one flatpak app matches a vague search, show a quick
// numbered list instead of just failing or guessing wrong. matches is
// (app_id, label) pairs
fn pick_from_matches(matches: &[(String, String)], noconfirm: bool, action: &str) -> Option<String> {
    if matches.is_empty() {
        return None;
    }
    if matches.len() == 1 {
        return Some(matches[0].0.clone());
    }
 
    println!();
    println!("{}", bold(&format!("A few Flatpak apps match - which one do you want to {}?", action)));
    for (i, (app_id, label)) in matches.iter().enumerate() {
        println!("  {} {}  {}", yellow(&format!("{})", i + 1)), label, dim(app_id));
    }
 
    if noconfirm {
        // can't prompt with --noconfirm on, and guessing which one someone
        // meant would be worse than just refusing
        return None;
    }
 
    print!("{}", bold("Choose a number: "));
    io::stdout().flush().ok();
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).ok();
    let idx: usize = choice.trim().parse().ok()?;
    if idx >= 1 && idx <= matches.len() {
        Some(matches[idx - 1].0.clone())
    } else {
        None
    }
}
 
// list of every currently-installed flatpak app, as (app_id, display_name)
fn flatpak_installed_apps() -> Vec<(String, String)> {
    if !flatpak_available() {
        return Vec::new();
    }
    let out = Command::new("flatpak").args(["list", "--app", "--columns=application,name"]).output();
    let text = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return Vec::new(),
    };
    text.lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() >= 2 {
                Some((cols[0].trim().to_string(), cols[1].trim().to_string()))
            } else {
                None
            }
        })
        .collect()
}
 
// fuzzy version of the old exact-match lookup - finds installed flatpak
// apps whose id or name loosely matches what was typed, then lets the
// person pick if there's more than one candidate
fn find_installed_flatpak_matches(term: &str) -> Vec<(String, String)> {
    let norm_term = normalize(term);
    flatpak_installed_apps()
        .into_iter()
        .filter(|(app_id, name)| {
            let id_norm = normalize(app_id);
            let name_norm = normalize(name);
            id_norm.contains(&norm_term) || norm_term.contains(&id_norm) || name_norm.contains(&norm_term) || norm_term.contains(&name_norm)
        })
        .map(|(app_id, name)| (app_id.clone(), name))
        .collect()
}
 
fn install_via_flatpak(app_id: &str, noconfirm: bool) {
    flatpak_ensure_flathub(noconfirm);
    if !confirm(&format!("Install '{}' via Flatpak?", app_id), true, noconfirm) {
        log(&format!("Skipped '{}'.", app_id), '-');
        return;
    }
    run_retry(&["flatpak", "install", "-y", "flathub", app_id], None, true, 3);
    log(&format!("'{}' installed via Flatpak.", app_id), '+');
    finish(&format!("'{}' is installed and ready to use.", app_id));
}
 
// this is basically the main install logic. tries apt first, only
// bothers building from source if it has to
fn install_target(
    target: &str,
    build_dir: &Path,
    skip_deps: bool,
    no_cache: bool,
    noconfirm: bool,
    force_build: bool,
    force_flatpak: bool,
) {
    // check if its already installed before going through any of the
    // source-decision logic below - no point asking "install from apt?"
    // for something already sitting on the system
    if !is_git_url(target) && !force_flatpak {
        if is_apt_installed(target) {
            if !confirm(&format!("'{}' is already installed (apt). Reinstall anyway?", target), false, noconfirm) {
                log(&format!("Skipping - '{}' is already installed.", target), '-');
                return;
            }
        } else if !force_build {
            let installed_flatpak = find_installed_flatpak_matches(target);
            if let Some((app_id, _)) = installed_flatpak.first() {
                if !confirm(&format!("'{}' is already installed via Flatpak ({}). Reinstall anyway?", target, app_id), false, noconfirm) {
                    log(&format!("Skipping - '{}' is already installed.", target), '-');
                    return;
                }
            }
        }
    }
 
    // --flatpak means skip apt entirely and go straight to flathub, even
    // if apt has a binary too. useful when the apt version is old/broken
    // or you just prefer sandboxed flatpak apps for certain things
    if force_flatpak {
        let matches = find_flatpak_search_matches(target);
        match pick_from_matches(&matches, noconfirm, "install") {
            Some(app_id) => install_via_flatpak(&app_id, noconfirm),
            None if matches.is_empty() => {
                fail_banner(&format!("couldn't find '{}' on Flathub", target));
                log(&format!("'{}' wasn't found on Flathub.", target), '-');
            }
            None => log("No selection made - not installing anything.", '-'),
        }
        return;
    }
 
    if !is_git_url(target) && !force_build && package_binary_available(target) {
        // apt has it - but check if flatpak ALSO has it before just going
        // with apt automatically. if both exist, let the person pick
        // instead of silently deciding for them
        let flatpak_matches = find_flatpak_search_matches(target);
 
        if let Some((app_id, _)) = flatpak_matches.first() {
            if noconfirm {
                // no prompt possible with --noconfirm, just go with apt as
                // the sensible default rather than silently picking flatpak
                run(&["apt-get", "install", "-y", target], None, true);
                log(&format!("'{}' installed from apt. No build needed.", target), '+');
                finish(&format!("'{}' is installed and ready to use.", target));
                return;
            }
 
            println!();
            println!("{}", bold(&format!("'{}' is available from multiple sources:", target)));
            println!("  {} apt (system package)", yellow("1)"));
            println!("  {} Flatpak ({})", yellow("2)"), app_id);
            print!("{}", bold("Choose a source (Enter = apt): "));
            io::stdout().flush().ok();
 
            let mut choice = String::new();
            io::stdin().read_line(&mut choice).ok();
 
            if choice.trim() == "2" {
                install_via_flatpak(app_id, noconfirm);
            } else {
                // picking "1" (or just hitting Enter) IS the confirmation -
                // no need to ask "install from apt?" again right after
                run(&["apt-get", "install", "-y", target], None, true);
                log(&format!("'{}' installed from apt. No build needed.", target), '+');
                finish(&format!("'{}' is installed and ready to use.", target));
            }
            return;
        }
 
        if confirm(&format!("Install '{}' from apt (prebuilt package)?", target), true, noconfirm) {
            run(&["apt-get", "install", "-y", target], None, true);
            log(&format!("'{}' installed from apt. No build needed.", target), '+');
            finish(&format!("'{}' is installed and ready to use.", target));
        } else {
            log(&format!("Skipped '{}'.", target), '-');
        }
        return;
    }
 
    // apt doesn't have it - check flatpak before giving up and compiling.
    // a lot of stuff (especially GUI apps) is only on flathub or the apt
    // version is way out of date, so this is worth checking first
    if !is_git_url(target) && !force_build {
        let matches = find_flatpak_search_matches(target);
        if !matches.is_empty() {
            log(&format!("No apt binary for '{}', but found something on Flatpak.", target), '*');
            if let Some(app_id) = pick_from_matches(&matches, noconfirm, "install") {
                install_via_flatpak(&app_id, noconfirm);
                return;
            }
            // fell through with no pick - drop into the source-build path below
        }
    }
 
    if !is_git_url(target) && !package_binary_available(target) {
        log(&format!("No prebuilt package found for '{}' - building from source instead.", target), '*');
    }
 
    build_from_source(target, build_dir, skip_deps, no_cache, noconfirm);
}
 
fn build_from_source(target: &str, build_dir: &Path, skip_deps: bool, no_cache: bool, noconfirm: bool) {
    let mut state = load_state();
    let source_kind = if is_git_url(target) { "git" } else { "apt" };
 
    let (name, pkg_dir): (String, PathBuf) = if source_kind == "git" {
        let stem = Path::new(target)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| target.to_string());
        let name = if stem.ends_with(".git") { stem[..stem.len() - 4].to_string() } else { stem };
        let dest = build_dir.join("src");
        let pkg_dir = clone_source(target, &dest);
        (name, pkg_dir)
    } else {
        fs::create_dir_all(build_dir).ok();
        let pkg_dir = fetch_debian_source(target, build_dir);
        (target.to_string(), pkg_dir)
    };
 
    let version = get_pkg_version(&pkg_dir);
    let verstr = version.clone().unwrap_or_else(|| "unversioned".to_string());
    let cached_dir = cache_dir().join(format!("{}-{}", name, verstr));
 
    if !no_cache && version.is_some() && cached_dir.exists() {
        let mut cached_debs: Vec<PathBuf> = fs::read_dir(&cached_dir)
            .map(|it| {
                it.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().map(|e| e == "deb").unwrap_or(false))
                    .collect()
            })
            .unwrap_or_default();
        cached_debs.sort();
        if !cached_debs.is_empty()
            && confirm(&format!("Cached build found for {} {}. Use it?", name, verstr), true, noconfirm)
        {
            install_debs(&cached_debs);
            state_upsert(
                &mut state,
                &name,
                PkgEntry {
                    source_kind: source_kind.to_string(),
                    source: target.to_string(),
                    version: version.clone(),
                    pkg_dir: pkg_dir.to_string_lossy().to_string(),
                    built_debs: cached_debs.iter().map(|d| d.to_string_lossy().to_string()).collect(),
                    installed_at: now_iso(),
                },
            );
            save_state(&state);
            finish(&format!("'{}' is installed (used the cached build, no rebuild needed).", name));
            return;
        }
    }
 
    if !confirm(&format!("Proceed with building '{}'?", name), true, noconfirm) {
        log(&format!("Skipped '{}'.", name), '-');
        return;
    }
 
    if !skip_deps {
        install_build_dependencies(&pkg_dir);
    }
 
    build_deb_package(&pkg_dir);
    let debs = collect_built_debs(&pkg_dir);
    let cached_debs = cache_debs(&name, &version, &debs);
    install_debs(&cached_debs);
    state_upsert(
        &mut state,
        &name,
        PkgEntry {
            source_kind: source_kind.to_string(),
            source: target.to_string(),
            version,
            pkg_dir: pkg_dir.to_string_lossy().to_string(),
            built_debs: cached_debs.iter().map(|d| d.to_string_lossy().to_string()).collect(),
            installed_at: now_iso(),
        },
    );
    save_state(&state);
    log(&format!("'{}' built and installed. That's buzz for ya.", name), '+');
    finish(&format!("'{}' is installed and ready to use.", name));
 
    let mut do_cleanup = false;
    if !skip_deps {
        do_cleanup = confirm("Remove build dependencies now?", false, noconfirm);
    }
    if do_cleanup {
        run(&["apt-get", "autoremove", "-y"], None, true);
        log("Build dependencies removed.", '+');
    }
}
 
// -- search + interactive picker --
 
fn apt_search_raw(term: &str) -> Vec<(String, String)> {
    let out = run_capture(&["apt-cache", "search", term], None);
    let mut entries = Vec::new();
    for line in out.lines() {
        if let Some(idx) = line.find(" - ") {
            let name = line[..idx].trim().to_string();
            let desc = line[idx + 3..].trim().to_string();
            entries.push((name, desc));
        } else if !line.trim().is_empty() {
            entries.push((line.trim().to_string(), String::new()));
        }
    }
    entries
}
 
// apt-cache search matches the FULL description, not just the name, so a
// short term like "obs" pulls in "Observer pattern", "observation", etc -
// anything with those letters buried in an unrelated word. narrow down to
// packages whose NAME actually relates to the term, same normalize()
// trick used for flatpak matching. falls back to the raw list if narrowing
// would leave nothing, in case someone genuinely searched by description
fn find_apt_search_matches(term: &str) -> Vec<(String, String)> {
    let norm_term = normalize(term);
    let all = apt_search_raw(term);
    let narrowed: Vec<(String, String)> = all
        .iter()
        .filter(|(name, _)| {
            let n = normalize(name);
            n.contains(&norm_term) || norm_term.contains(&n)
        })
        .cloned()
        .collect();
    if narrowed.is_empty() {
        all
    } else {
        narrowed
    }
}
 
fn parse_selection(selection: &str, count: usize) -> BTreeSet<usize> {
    let selection = selection.trim();
    if selection.is_empty() {
        return BTreeSet::new();
    }
    let mut positive: BTreeSet<usize> = BTreeSet::new();
    let mut exclude: BTreeSet<usize> = BTreeSet::new();
 
    for tok in selection.split_whitespace() {
        let (is_excl, body) = match tok.strip_prefix('^') {
            Some(rest) => (true, rest),
            None => (false, tok),
        };
        let set: BTreeSet<usize> = if let Some(dash) = body.find('-') {
            let a = &body[..dash];
            let b = &body[dash + 1..];
            match (a.parse::<usize>(), b.parse::<usize>()) {
                (Ok(a), Ok(b)) if a <= b => (a..=b).collect(),
                _ => BTreeSet::new(),
            }
        } else {
            match body.parse::<usize>() {
                Ok(n) => {
                    let mut s = BTreeSet::new();
                    s.insert(n);
                    s
                }
                Err(_) => BTreeSet::new(),
            }
        };
        if is_excl {
            exclude.extend(set);
        } else {
            positive.extend(set);
        }
    }
 
    if positive.is_empty() && !exclude.is_empty() {
        positive = (1..=count).collect();
    }
 
    positive.difference(&exclude).cloned().filter(|&i| i >= 1 && i <= count).collect()
}
 
struct Entry {
    kind: &'static str,
    name: String,
    extra: String,
}
 
fn cmd_interactive(
    term: &str,
    noconfirm: bool,
    build_dir: &Path,
    skip_deps: bool,
    no_cache: bool,
    force_build: bool,
    force_flatpak: bool,
) {
    log(&format!("Searching for '{}' ...", term), '+');
    let apt_results = find_apt_search_matches(term);
    let flatpak_results = find_flatpak_matches_full(term); // empty vec if flatpak isn't installed, no crash
    let state = load_state();
    let term_lower = term.to_lowercase();
    let buzz_results: Vec<(String, String)> = state
        .iter()
        .filter(|(n, _)| n.to_lowercase().contains(&term_lower))
        .map(|(n, e)| (n.clone(), e.version.clone().unwrap_or_default()))
        .collect();
 
    let mut combined: Vec<Entry> = Vec::new();
    for (n, d) in apt_results {
        combined.push(Entry { kind: "apt", name: n, extra: d });
    }
    for (n, v) in buzz_results {
        combined.push(Entry { kind: "buzz", name: n, extra: v });
    }
    for (app_id, name, desc) in flatpak_results {
        // extra shows the human readable name since app ids like
        // org.gimp.GIMP arent super readable on their own
        combined.push(Entry { kind: "flatpak", name: app_id, extra: format!("{} - {}", name, desc) });
    }
 
    if combined.is_empty() {
        log("No results found.", '-');
        return;
    }
 
    println!();
    let count = combined.len();
    for (idx, entry) in combined.iter().enumerate().rev() {
        let real_idx = idx + 1;
        let tag = match entry.kind {
            "buzz" => cyan("[buzz]"),
            "flatpak" => magenta("[flatpak]"),
            _ => dim("[apt]"),
        };
        println!("{}  {} {}  {}", yellow(&real_idx.to_string()), tag, green(&entry.name), dim(&entry.extra));
    }
    println!();
 
    print!("{}", bold("==> Packages to install (eg: 1 2 3, 1-3, ^4): "));
    io::stdout().flush().ok();
    let mut raw = String::new();
    io::stdin().read_line(&mut raw).ok();
    let picks = parse_selection(raw.trim(), count);
    if picks.is_empty() {
        log("Nothing selected.", '-');
        return;
    }
 
    // gotta keep the kind attached to each pick now, not just the name,
    // since flatpak stuff needs to go down a totally different path
    let targets: Vec<(&'static str, String)> =
        picks.iter().map(|&i| (combined[i - 1].kind, combined[i - 1].name.clone())).collect();
    let names_only: Vec<&str> = targets.iter().map(|(_, n)| n.as_str()).collect();
    log(&format!("About to install: {}", names_only.join(", ")), '+');
    if !confirm("Proceed with installation?", true, noconfirm) {
        log("Aborted.", '-');
        return;
    }
 
    for (kind, name) in targets {
        if kind == "flatpak" {
            install_via_flatpak(&name, noconfirm);
        } else {
            install_target(&name, build_dir, skip_deps, no_cache, noconfirm, force_build, force_flatpak);
        }
    }
}
 
// -- commands --
 
fn cmd_search_cmd(term: &str) {
    log(&format!("Searching apt for '{}' ...", term), '+');
    for (name, desc) in find_apt_search_matches(term) {
        println!("  {}  {}", green(&name), dim(&desc));
    }
 
    let flatpak_matches = find_flatpak_matches_full(term);
    if !flatpak_matches.is_empty() {
        println!();
        log("Matching Flatpak apps:", '+');
        for (app_id, name, desc) in flatpak_matches {
            println!("  {} {}  ({})  {}", magenta("[flatpak]"), green(&app_id), name, dim(&desc));
        }
    }
 
    let state = load_state();
    let term_lower = term.to_lowercase();
    let matches: Vec<&(String, PkgEntry)> =
        state.iter().filter(|(n, _)| n.to_lowercase().contains(&term_lower)).collect();
    if !matches.is_empty() {
        println!();
        log("Matching buzz-tracked source builds:", '+');
        for (name, entry) in matches {
            let ver = entry.version.clone().unwrap_or_else(|| "unversioned".to_string());
            println!("  {} {}  ({})  <- {}", cyan("[buzz]"), green(name), ver, entry.source);
        }
    }
}
 
// shows details before you commit to installing something, instead of
// going in blind off a one-line search description
fn cmd_info(term: &str) {
    log(&format!("Looking up '{}' ...", term), '+');
    let mut found_anything = false;
 
    if package_binary_available(term) {
        found_anything = true;
        println!();
        log("apt package info:", '+');
        if let Ok(o) = Command::new("apt-cache").args(["show", term]).output() {
            if o.status.success() {
                // apt-cache show already formats this nicely, just print it as-is
                print!("{}", String::from_utf8_lossy(&o.stdout));
            }
        }
    }
 
    if flatpak_available() {
        // use the same fuzzy matching as install/remove so `buzz info obs`
        // works the same way `buzz install obs` does, instead of demanding
        // the exact app id
        let matches = find_flatpak_search_matches(term);
        if let Some((app_id, _)) = matches.first() {
            found_anything = true;
            println!();
            log(&format!("Flatpak info ({}):", app_id), '+');
            if let Ok(o) = Command::new("flatpak").args(["remote-info", "--log", "flathub", app_id]).output() {
                if o.status.success() {
                    print!("{}", String::from_utf8_lossy(&o.stdout));
                } else {
                    // flathub remote might not be added yet, not a big deal,
                    // just say so instead of silently printing nothing
                    log("(couldn't fetch flatpak details - is the flathub remote set up?)", '*');
                }
            }
        }
    }
 
    if !found_anything {
        log(&format!("No info found for '{}' in apt or Flatpak.", term), '-');
    }
}
 
fn cmd_upgrade(noconfirm: bool) {
    log("Updating apt package lists and upgrading system packages ...", '+');
    run_retry(&["apt-get", "update"], None, true, 3);
    run(&["apt-get", "upgrade", "-y"], None, true);
 
    let mut state = load_state();
    if state.is_empty() {
        finish("Upgrade finished - your system is up to date.");
        return;
    }
 
    log("Checking buzz-tracked source packages for new versions ...", '+');
    let names: Vec<String> = state.iter().map(|(n, _)| n.clone()).collect();
 
    for name in names {
        let entry = match state_find(&state, &name) {
            Some(e) => e.clone(),
            None => continue,
        };
 
        let pkg_dir: PathBuf = if entry.source_kind == "apt" {
            let build_root = default_build_dir().join(format!("upgrade-{}", name));
            fetch_debian_source(&name, &build_root)
        } else {
            let existing = PathBuf::from(&entry.pkg_dir);
            if existing.exists() {
                log(&format!("Pulling latest for '{}' ...", name), '+');
                run(&["git", "pull"], Some(&existing), false);
                existing
            } else {
                let dest = default_build_dir().join(format!("upgrade-{}", name)).join("src");
                clone_source(&entry.source, &dest)
            }
        };
 
        let new_version = get_pkg_version(&pkg_dir);
 
        if entry.source_kind == "apt" && new_version == entry.version {
            log(
                &format!("'{}' is already up to date ({}).", name, entry.version.clone().unwrap_or_default()),
                '+',
            );
            continue;
        }
 
        let old_v = entry.version.clone().unwrap_or_else(|| "?".to_string());
        let new_v = new_version.clone().unwrap_or_else(|| "?".to_string());
        // color-coded version diff so the change is obvious at a glance
        // instead of squinting at two similar-looking version strings
        let diff = format!("{} {} {}", red(&old_v), dim("->"), green(&new_v));
        if !confirm(&format!("Rebuild '{}' ({})?", name, diff), true, noconfirm) {
            continue;
        }
 
        install_build_dependencies(&pkg_dir);
        build_deb_package(&pkg_dir);
        let debs = collect_built_debs(&pkg_dir);
        let cached_debs = cache_debs(&name, &new_version, &debs);
        install_debs(&cached_debs);
        state_upsert(
            &mut state,
            &name,
            PkgEntry {
                source_kind: entry.source_kind.clone(),
                source: entry.source.clone(),
                version: new_version,
                pkg_dir: pkg_dir.to_string_lossy().to_string(),
                built_debs: cached_debs.iter().map(|d| d.to_string_lossy().to_string()).collect(),
                installed_at: now_iso(),
            },
        );
        save_state(&state);
        // confirm each package finished before moving to the next one,
        // otherwise it just looks like it silently jumped from build output
        // straight into the next package with no closure
        log(&format!("'{}' updated: {}", name, diff), '+');
    }
 
    finish("Upgrade finished - your system and tracked packages are up to date.");
}
 
fn cmd_remove(name: &str, purge: bool, purge_cache: bool, noconfirm: bool) {
    // check apt first since thats the common case, only look at flatpak
    // if apt doesnt know about it
    if is_apt_installed(name) {
        let verb = if purge { "purge" } else { "remove" };
        let mut chars = verb.chars();
        let verb_cap = match chars.next() {
            Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        };
 
        if !confirm(&format!("{} '{}'?", verb_cap, name), true, noconfirm) {
            log("Aborted.", '-');
            return;
        }
        run(&["apt-get", verb, "-y", name], None, true);
 
        let mut state = load_state();
        state_remove(&mut state, name);
        save_state(&state);
 
        // this was the actual missing piece - after all that apt output
        // scrolls by, say plainly that it worked instead of just falling
        // through to the next prompt with nothing
        let past_tense = if purge { "purged" } else { "removed" };
        log(&format!("'{}' was {} successfully.", name, past_tense), '+');
        finish(&format!("'{}' has been {}.", name, past_tense));
 
        if purge_cache {
            if let Ok(entries) = fs::read_dir(cache_dir()) {
                let prefix = format!("{}-", name);
                for e in entries.filter_map(|e| e.ok()) {
                    let fname = e.file_name().to_string_lossy().to_string();
                    if fname.starts_with(&prefix) {
                        fs::remove_dir_all(e.path()).ok();
                    }
                }
            }
            log(&format!("Purged cached builds for '{}'.", name), '+');
        }
        return;
    }
 
    let flatpak_matches = find_installed_flatpak_matches(name);
    if let Some(app_id) = pick_from_matches(&flatpak_matches, noconfirm, "remove") {
        if !confirm(&format!("Remove Flatpak app '{}'?", app_id), true, noconfirm) {
            log("Aborted.", '-');
            return;
        }
        run(&["flatpak", "uninstall", "-y", &app_id], None, true);
        log(&format!("'{}' removed via Flatpak.", app_id), '+');
        finish(&format!("'{}' has been removed.", app_id));
        return;
    }
 
    if !flatpak_matches.is_empty() {
        // pick_from_matches returned None with candidates present - either
        // noconfirm blocked the prompt or the person didn't pick a valid number
        log("No selection made - nothing removed.", '-');
        return;
    }
 
    log(&format!("'{}' doesn't appear to be installed via apt or Flatpak.", name), '-');
}
 
// shows everything buzz knows about - source builds it tracks, plus a
// summary of flatpak apps, so you don't have to cat state.json by hand
fn cmd_list() {
    let state = load_state();
 
    println!();
    if state.is_empty() {
        log("No source-built packages tracked by buzz.", '*');
    } else {
        log("Source packages built by buzz:", '+');
        for (name, entry) in &state {
            let ver = entry.version.clone().unwrap_or_else(|| "unversioned".to_string());
            let installed = if is_apt_installed(name) { green("installed") } else { red("not installed") };
            println!(
                "  {} {}  ({})  [{}]  <- {}",
                cyan("[buzz]"),
                green(name),
                ver,
                installed,
                dim(&entry.source)
            );
        }
    }
 
    let flatpaks = flatpak_installed_apps();
    if !flatpaks.is_empty() {
        println!();
        log("Flatpak apps installed on this system:", '+');
        for (app_id, name) in &flatpaks {
            println!("  {} {}  ({})", magenta("[flatpak]"), green(app_id), dim(name));
        }
    }
 
    println!();
    log(
        &format!(
            "{} source build(s) tracked, {} flatpak app(s) installed.",
            state.len(),
            flatpaks.len()
        ),
        '*',
    );
}
 
fn cmd_clean(yes: bool) {
    if !confirm(
        &format!("Delete {} and {}?", default_build_dir().display(), cache_dir().display()),
        false,
        yes,
    ) {
        log("Aborted.", '-');
        return;
    }
    fs::remove_dir_all(default_build_dir()).ok();
    fs::remove_dir_all(cache_dir()).ok();
    finish("All cleaned up - build cache and temp files are gone.");
}
 
// rebuilds and reinstalls buzz itself. has to be run from inside the git
// checkout (or given the path to one) since the installed binary has no
// idea where its own source lives - it's just a file in /usr/local/bin
// at that point
fn cmd_self_update(repo_path: Option<String>, noconfirm: bool) {
    let repo_dir = match repo_path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };
 
    let cargo_toml = repo_dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        die(&format!(
            "no Cargo.toml found in {} - run this from inside your buzz checkout, \
             or pass the path: buzz self-update /path/to/buzz",
            repo_dir.display()
        ));
    }
 
    if repo_dir.join(".git").exists() {
        log("Pulling latest changes ...", '+');
        run_retry(&["git", "pull"], Some(&repo_dir), false, 3);
    } else {
        log("(not a git checkout, skipping pull - just rebuilding what's already on disk)", '*');
    }
 
    if !confirm("Rebuild and reinstall buzz now?", true, noconfirm) {
        log("Skipped.", '-');
        return;
    }
 
    log("Building buzz (release) ...", '+');
    run(&["cargo", "build", "--release"], Some(&repo_dir), false);
 
    let built_bin = repo_dir.join("target").join("release").join("buzz");
    if !built_bin.exists() {
        die("build finished but the binary wasn't where it should be - something went wrong");
    }
 
    run(&["cp", &built_bin.to_string_lossy(), "/usr/local/bin/buzz"], None, true);
    run(&["chmod", "755", "/usr/local/bin/buzz"], None, true);
    finish("buzz has been rebuilt and reinstalled.");
}
 
fn cmd_passthrough(argv: &[String]) {
    let mut cmd: Vec<&str> = vec!["apt-get"];
    for a in argv {
        cmd.push(a.as_str());
    }
    run(&cmd, None, true);
    // run() already exits the whole program on failure, so if we get here
    // it actually worked - say so instead of just dropping back to the
    // prompt with no closing sentence
    log("Command finished successfully.", '+');
}
 
// -- per-subcommand arg parsing (no clap - just walk the tokens) --
 
fn cmd_install_cli(args: &[String]) {
    let mut target: Option<String> = None;
    let mut build_dir = default_build_dir();
    let mut skip_deps = false;
    let mut no_cache = false;
    let mut noconfirm = false;
    let mut force_build = false;
    let mut force_flatpak = false;
 
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--build-dir" => {
                i += 1;
                if i < args.len() {
                    build_dir = PathBuf::from(&args[i]);
                }
            }
            "--skip-deps" => skip_deps = true,
            "--no-cache" => no_cache = true,
            "--noconfirm" | "-y" => noconfirm = true,
            "--build" => force_build = true,
            "--flatpak" => force_flatpak = true,
            other => {
                if target.is_none() {
                    target = Some(other.to_string());
                }
            }
        }
        i += 1;
    }
 
    if force_build && force_flatpak {
        die("--build and --flatpak can't both be used - pick one source to force");
    }
 
    let target = target.unwrap_or_else(|| die("install requires a target (package name or git URL)"));
    install_target(&target, &build_dir, skip_deps, no_cache, noconfirm, force_build, force_flatpak);
}
 
fn cmd_search_cli(args: &[String]) {
    let term = args.first().cloned().unwrap_or_else(|| die("search requires a term"));
    cmd_search_cmd(&term);
}
 
fn cmd_info_cli(args: &[String]) {
    let term = args.first().cloned().unwrap_or_else(|| die("info requires a package name"));
    cmd_info(&term);
}
 
fn cmd_upgrade_cli(args: &[String]) {
    let noconfirm = args.iter().any(|a| a == "--noconfirm" || a == "-y");
    cmd_upgrade(noconfirm);
}
 
fn cmd_remove_cli(args: &[String]) {
    let mut name: Option<String> = None;
    let mut purge = false;
    let mut purge_cache = false;
    let mut noconfirm = false;
 
    for a in args {
        match a.as_str() {
            "--purge" => purge = true,
            "--purge-cache" => purge_cache = true,
            "--noconfirm" | "-y" => noconfirm = true,
            other => {
                if name.is_none() {
                    name = Some(other.to_string());
                }
            }
        }
    }
 
    let name = name.unwrap_or_else(|| die("remove requires a package name"));
    cmd_remove(&name, purge, purge_cache, noconfirm);
}
 
fn cmd_clean_cli(args: &[String]) {
    let yes = args.iter().any(|a| a == "-y" || a == "--yes");
    cmd_clean(yes);
}
 
fn cmd_self_update_cli(args: &[String]) {
    let noconfirm = args.iter().any(|a| a == "--noconfirm" || a == "-y");
    let repo_path = args.iter().find(|a| !a.starts_with('-')).cloned();
    cmd_self_update(repo_path, noconfirm);
}
 
fn print_help() {
    println!("buzz - a yay/paru-style package manager for Debian\n");
    println!("USAGE:");
    println!("  buzz <term>              interactive search + install picker");
    println!("                            (searches apt AND flatpak, if installed)");
    println!("  buzz install <target>    apt binary if one exists, else flatpak, else source");
    println!("  buzz search <term>       plain search (apt + flatpak)");
    println!("  buzz info <name>         show details before installing (apt + flatpak)");
    println!("  buzz upgrade              apt upgrade + rebuild tracked source packages");
    println!("  buzz remove <name>       removes it wherever it's actually installed from");
    println!("                            (apt or flatpak, whichever applies)");
    println!("  buzz clean                wipe build dir and .deb cache");
    println!("  buzz list                 show buzz-tracked builds + installed flatpaks");
    println!("  buzz self-update [path]  rebuild + reinstall buzz itself (run from the");
    println!("                            buzz checkout, or pass its path)");
    println!("  buzz <apt-verb> ...      passthrough to apt-get (update, autoremove, ...)");
    println!("\nFlags:");
    println!("  --noconfirm, -y   skip confirmation prompts");
    println!("  --build        force building from source even if a binary exists");
    println!("  --flatpak      force installing via Flatpak, skipping apt entirely");
    println!("\nNote: flatpak search/install only works if flatpak is installed.");
    println!("      buzz won't install flatpak itself - grab it with your distro's");
    println!("      normal package manager first (e.g. apt install flatpak).");
    println!("      flatpak installs via buzz are system-wide (sudo), not per-user.");
}
 
// -- entrypoint --
 
fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        print_help();
        return;
    }
 
    let first = argv[0].clone();
 
    if first == "-h" || first == "--help" {
        print_help();
        return;
    }
 
    match first.as_str() {
        "install" => return cmd_install_cli(&argv[1..]),
        "search" => return cmd_search_cli(&argv[1..]),
        "info" => return cmd_info_cli(&argv[1..]),
        "upgrade" => return cmd_upgrade_cli(&argv[1..]),
        "remove" => return cmd_remove_cli(&argv[1..]),
        "clean" => return cmd_clean_cli(&argv[1..]),
        "list" => return cmd_list(),
        "self-update" => return cmd_self_update_cli(&argv[1..]),
        _ => {}
    }
 
    if APT_ONLY_VERBS.contains(&first.as_str()) || first.starts_with('-') {
        cmd_passthrough(&argv);
        return;
    }
 
    // bare word(s), not a known command or apt verb -> yay-style interactive search
    let noconfirm = argv.iter().any(|a| a == "--noconfirm" || a == "-y");
    let force_build = argv.iter().any(|a| a == "--build");
    let force_flatpak = argv.iter().any(|a| a == "--flatpak");
    let term_words: Vec<&str> = argv
        .iter()
        .filter(|a| {
            let s = a.as_str();
            s != "--noconfirm" && s != "-y" && s != "--build" && s != "--flatpak"
        })
        .map(|s| s.as_str())
        .collect();
    let term = term_words.join(" ");
    cmd_interactive(&term, noconfirm, &default_build_dir(), false, false, force_build, force_flatpak);
}
 
// -- tests (run with `cargo test`) --
 
#[cfg(test)]
mod tests {
    use super::*;
 
    #[test]
    fn selection_basic() {
        assert_eq!(parse_selection("1 2 3", 10), [1, 2, 3].into_iter().collect());
    }
 
    #[test]
    fn selection_range() {
        assert_eq!(parse_selection("1-3", 10), [1, 2, 3].into_iter().collect());
    }
 
    #[test]
    fn selection_range_with_exclude() {
        assert_eq!(parse_selection("1-5 ^3", 10), [1, 2, 4, 5].into_iter().collect());
    }
 
    #[test]
    fn selection_exclude_only_defaults_to_all() {
        assert_eq!(parse_selection("^4", 5), [1, 2, 3, 5].into_iter().collect());
    }
 
    #[test]
    fn selection_empty() {
        assert_eq!(parse_selection("", 10), BTreeSet::new());
    }
 
    #[test]
    fn selection_out_of_range_dropped() {
        assert_eq!(parse_selection("7", 5), BTreeSet::new());
    }
 
    #[test]
    fn json_roundtrip() {
        let mut state: State = Vec::new();
        state.push((
            "nginx".to_string(),
            PkgEntry {
                source_kind: "apt".to_string(),
                source: "nginx".to_string(),
                version: Some("1.23-1".to_string()),
                pkg_dir: "/tmp/nginx".to_string(),
                built_debs: vec!["/tmp/nginx.deb".to_string()],
                installed_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ));
 
        let mut pkg_pairs = Vec::new();
        for (name, entry) in &state {
            let obj = vec![
                ("source_kind".to_string(), Json::Str(entry.source_kind.clone())),
                ("source".to_string(), Json::Str(entry.source.clone())),
                ("version".to_string(), Json::Str(entry.version.clone().unwrap())),
                ("pkg_dir".to_string(), Json::Str(entry.pkg_dir.clone())),
                ("built_debs".to_string(), Json::Arr(entry.built_debs.iter().map(|d| Json::Str(d.clone())).collect())),
                ("installed_at".to_string(), Json::Str(entry.installed_at.clone())),
            ];
            pkg_pairs.push((name.clone(), Json::Obj(obj)));
        }
        let root = Json::Obj(vec![("packages".to_string(), Json::Obj(pkg_pairs))]);
        let text = json_to_string(&root, 0);
 
        let parsed = parse_json(&text);
        let nginx = parsed.get("packages").unwrap().get("nginx").unwrap();
        assert_eq!(nginx.get("source_kind").unwrap().as_str(), Some("apt"));
        assert_eq!(nginx.get("version").unwrap().as_str(), Some("1.23-1"));
        assert_eq!(
            nginx.get("built_debs").unwrap().as_arr().unwrap()[0].as_str(),
            Some("/tmp/nginx.deb")
        );
    }
}
