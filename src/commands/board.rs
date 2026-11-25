//! Repository project boards: Gitea's `projects` API, addressed by issue
//! number and column title instead of the internal IDs the routes take.
use std::fmt::Write;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::args::{Args, BoardArgs, BoardSelector, ListState, PageArgs};
use crate::error::Error;
use crate::output::{Outcome, plain_field};

use super::typed::{RepoClient, collect_arrays, reject_read_flags, reject_yes};

const EVERY_PAGE: PageArgs = PageArgs {
    page: 1,
    all: true,
    limit: 50,
};
const ONE_ITEM: PageArgs = PageArgs {
    page: 1,
    all: false,
    limit: 1,
};

#[derive(Deserialize)]
struct BoardResponse {
    id: u64,
    title: String,
    #[serde(default)]
    description: String,
    state: String,
    #[serde(default)]
    num_open_issues: u64,
    #[serde(default)]
    num_closed_issues: u64,
    #[serde(default)]
    html_url: String,
    created_at: Option<String>,
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct BoardRecord {
    kind: &'static str,
    id: u64,
    title: String,
    description: String,
    state: &'static str,
    open_issues: u64,
    closed_issues: u64,
    html_url: String,
    created_at: Option<String>,
    updated_at: Option<String>,
}

impl TryFrom<BoardResponse> for BoardRecord {
    type Error = Error;

    fn try_from(value: BoardResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            kind: "board",
            id: value.id,
            state: known_state(&value.state, "board")?,
            title: value.title,
            description: value.description,
            open_issues: value.num_open_issues,
            closed_issues: value.num_closed_issues,
            html_url: value.html_url,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }
}

#[derive(Deserialize)]
struct ColumnResponse {
    id: u64,
    title: String,
    #[serde(default)]
    default: bool,
}

/// A card is the issue or pull request it shows. The card routes take `id`,
/// the forge-wide issue ID, and return 404 for the repository's `number`.
#[derive(Deserialize)]
struct CardResponse {
    id: u64,
    number: u64,
    title: String,
    state: String,
    #[serde(default)]
    html_url: String,
    pull_request: Option<Value>,
}

#[derive(Serialize)]
struct CardRecord {
    number: u64,
    state: &'static str,
    title: String,
    pull_request: bool,
    html_url: String,
}

#[derive(Serialize)]
struct ColumnRecord {
    kind: &'static str,
    board: u64,
    id: u64,
    title: String,
    default: bool,
    cards: Vec<CardRecord>,
}

#[derive(Deserialize)]
struct IssueId {
    id: u64,
}

#[derive(Serialize)]
struct ResultRecord<'a> {
    kind: &'static str,
    action: &'static str,
    ok: bool,
    number: u64,
    board: u64,
    column: &'a str,
    column_id: u64,
}

pub(crate) fn run(common: &Args, args: &BoardArgs) -> Result<Outcome, Error> {
    match args {
        BoardArgs::List { state, paging } => list(common, *state, paging),
        BoardArgs::View { board } => view(common, board.as_ref()),
        BoardArgs::Move {
            number,
            column,
            board,
        } => place(common, *number, column, board.as_ref()),
        BoardArgs::Remove { number, board } => remove(common, *number, board.as_ref()),
    }
}

fn list(common: &Args, state: ListState, paging: &PageArgs) -> Result<Outcome, Error> {
    reject_read_flags(common, "board list")?;
    let repo = RepoClient::resolve(common)?;
    let records: Vec<BoardRecord> = boards(&repo, state.as_str(), paging)?
        .into_iter()
        .map(BoardRecord::try_from)
        .collect::<Result<_, _>>()?;
    if common.json {
        return Outcome::json(&records);
    }
    let mut text = String::new();
    for record in records {
        writeln!(
            &mut text,
            "{}\t{}\t{}\t{}\t{}\t{}",
            record.id,
            record.state,
            plain_field(&record.title),
            record.open_issues,
            record.closed_issues,
            plain_field(&record.html_url)
        )
        .map_err(format_error)?;
    }
    Ok(Outcome::text(text))
}

fn view(common: &Args, selector: Option<&BoardSelector>) -> Result<Outcome, Error> {
    reject_read_flags(common, "board view")?;
    let repo = RepoClient::resolve(common)?;
    let board = resolve(&repo, selector)?;
    let mut records = Vec::new();
    for column in columns(&repo, board.id)? {
        let cards = cards(&repo, board.id, column.id)?
            .into_iter()
            .map(|card| {
                Ok(CardRecord {
                    number: card.number,
                    state: known_state(&card.state, "issue")?,
                    title: card.title,
                    pull_request: card.pull_request.is_some_and(|value| !value.is_null()),
                    html_url: card.html_url,
                })
            })
            .collect::<Result<_, Error>>()?;
        records.push(ColumnRecord {
            kind: "column",
            board: board.id,
            id: column.id,
            title: column.title,
            default: column.default,
            cards,
        });
    }
    if common.json {
        return Outcome::json(&records);
    }
    // Each card line repeats its column title so one matching line answers
    // "where is issue N"; a column line also reports an empty column.
    let mut text = String::new();
    for column in records {
        writeln!(
            &mut text,
            "column\t{}\t{}\t{}",
            column.id,
            plain_field(&column.title),
            column.cards.len()
        )
        .map_err(format_error)?;
        for card in column.cards {
            writeln!(
                &mut text,
                "card\t{}\t{}\t{}\t{}",
                plain_field(&column.title),
                card.number,
                card.state,
                plain_field(&card.title)
            )
            .map_err(format_error)?;
        }
    }
    Ok(Outcome::text(text))
}

/// The forge's add-card route also moves a card that is already on the board,
/// so placing and moving are one request.
fn place(
    common: &Args,
    number: u64,
    column: &str,
    selector: Option<&BoardSelector>,
) -> Result<Outcome, Error> {
    reject_yes(common, "board move")?;
    let repo = RepoClient::resolve(common)?;
    let board = resolve(&repo, selector)?;
    let columns = columns(&repo, board.id)?;
    let titles: Vec<&str> = columns.iter().map(|column| column.title.as_str()).collect();
    let target = match match_title(&titles, column) {
        TitleMatch::One(index) => &columns[index],
        TitleMatch::None => {
            return Err(Error::usage(format!(
                "board {} has no column titled {:?}; columns: {}",
                board.id,
                column,
                quoted(&titles)
            )));
        }
        TitleMatch::Many => {
            return Err(Error::usage(format!(
                "board {} has more than one column titled {:?}; rename one on the forge",
                board.id, column
            )));
        }
    };
    let (issue, _) = repo.get::<IssueId>(&repo.path(&format!("issues/{number}")), "issue")?;
    let path = repo.path(&format!(
        "projects/{}/columns/{}/issues/{}",
        board.id, target.id, issue.id
    ));
    if common.dry_run {
        return repo.dry_run_empty(common.json, "POST", &path);
    }
    repo.client.send("POST", &path, None)?;
    result(common.json, "board.move", number, board.id, target)
}

fn remove(common: &Args, number: u64, selector: Option<&BoardSelector>) -> Result<Outcome, Error> {
    reject_yes(common, "board remove")?;
    let repo = RepoClient::resolve(common)?;
    let board = resolve(&repo, selector)?;
    // The remove route names the card's column, which only the columns report.
    let mut found = None;
    for column in columns(&repo, board.id)? {
        if let Some(card) = cards(&repo, board.id, column.id)?
            .into_iter()
            .find(|card| card.number == number)
        {
            found = Some((column, card));
            break;
        }
    }
    let (column, card) = found.ok_or_else(|| {
        Error::usage(format!(
            "issue #{number} has no card on board {} ({:?})",
            board.id,
            plain_field(&board.title).to_string()
        ))
    })?;
    let path = repo.path(&format!(
        "projects/{}/columns/{}/issues/{}",
        board.id, column.id, card.id
    ));
    if common.dry_run {
        return repo.dry_run_empty(common.json, "DELETE", &path);
    }
    repo.client.send("DELETE", &path, None)?;
    result(common.json, "board.remove", number, board.id, &column)
}

/// The board ID for `issue create --board`, checked before the issue is written.
pub(super) fn resolve_id(repo: &RepoClient, selector: &BoardSelector) -> Result<u64, Error> {
    resolve(repo, Some(selector)).map(|board| board.id)
}

/// Without a selector, a repository's only open board is the board.
fn resolve(repo: &RepoClient, selector: Option<&BoardSelector>) -> Result<BoardResponse, Error> {
    let name = repo.name();
    match selector {
        Some(BoardSelector::Id(id)) => {
            let path = repo.path(&format!("projects/{id}"));
            match repo.get::<BoardResponse>(&path, "board") {
                Ok((board, _)) => Ok(board),
                Err(error) if error.http_status() == Some(404) => {
                    // The list tells a missing board from a forge without boards.
                    boards(repo, "all", &ONE_ITEM)?;
                    Err(Error::http(
                        404,
                        format!("{name} has no board with ID {id} (HTTP 404)"),
                    ))
                }
                Err(error) => Err(error),
            }
        }
        Some(BoardSelector::Title(title)) => {
            let mut all = boards(repo, "all", &EVERY_PAGE)?;
            let titles: Vec<&str> = all.iter().map(|board| board.title.as_str()).collect();
            match match_title(&titles, title) {
                TitleMatch::One(index) => Ok(all.swap_remove(index)),
                TitleMatch::None => Err(Error::usage(format!(
                    "{name} has no board titled {title:?}; boards: {}",
                    described(&all)
                ))),
                TitleMatch::Many => Err(Error::usage(format!(
                    "{name} has more than one board titled {title:?}; pass --board-id ID: {}",
                    described(&all)
                ))),
            }
        }
        None => {
            let mut open = boards(repo, "open", &EVERY_PAGE)?;
            match open.len() {
                1 => Ok(open.swap_remove(0)),
                0 => Err(Error::usage(format!(
                    "{name} has no open board; pass --board TITLE or --board-id ID to use a closed one"
                ))),
                count => Err(Error::usage(format!(
                    "{name} has {count} open boards; pass --board TITLE or --board-id ID: {}",
                    described(&open)
                ))),
            }
        }
    }
}

fn boards(repo: &RepoClient, state: &str, paging: &PageArgs) -> Result<Vec<BoardResponse>, Error> {
    let path = repo.path(&format!("projects?state={state}"));
    collect_arrays(repo, &path, paging, "board list").map_err(|error| missing_api(repo, error))
}

fn columns(repo: &RepoClient, board: u64) -> Result<Vec<ColumnResponse>, Error> {
    // The forge returns columns in board order.
    let path = repo.path(&format!("projects/{board}/columns"));
    collect_arrays(repo, &path, &EVERY_PAGE, "board columns")
}

fn cards(repo: &RepoClient, board: u64, column: u64) -> Result<Vec<CardResponse>, Error> {
    let path = repo.path(&format!("projects/{board}/columns/{column}/issues"));
    collect_arrays(repo, &path, &EVERY_PAGE, "board cards")
}

/// Every board command reads the board list or a board first. When that route
/// answers 404 for a repository that exists, the forge does not serve it:
/// Forgejo through version 17 has no project-board API. That is an incompatible forge, not a
/// missing record, and it is reported before any write.
fn missing_api(repo: &RepoClient, error: Error) -> Error {
    if error.http_status() != Some(404) || repo.get::<Value>(&repo.root(), "repository").is_err() {
        return error;
    }
    Error::data(format!(
        "{} has no project-board API for {}: the board route returned HTTP 404 for a repository that exists. Forgejo through version 17 does not provide this API; on Gitea, check that projects are enabled for the repository",
        repo.client.host().as_str(),
        repo.name()
    ))
}

enum TitleMatch {
    One(usize),
    None,
    Many,
}

/// An exact title wins; otherwise titles match ignoring case. Either way the
/// match must be the only one.
fn match_title(titles: &[&str], wanted: &str) -> TitleMatch {
    let positions = |matches: &dyn Fn(&str) -> bool| -> Vec<usize> {
        titles
            .iter()
            .enumerate()
            .filter(|(_, title)| matches(title))
            .map(|(index, _)| index)
            .collect()
    };
    let mut found = positions(&|title| title == wanted);
    if found.is_empty() {
        let lowered = wanted.to_lowercase();
        found = positions(&|title| title.to_lowercase() == lowered);
    }
    match found.as_slice() {
        [index] => TitleMatch::One(*index),
        [] => TitleMatch::None,
        _ => TitleMatch::Many,
    }
}

fn quoted(titles: &[&str]) -> String {
    if titles.is_empty() {
        return "none".to_owned();
    }
    titles
        .iter()
        .map(|title| format!("{:?}", plain_field(title).to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn described(boards: &[BoardResponse]) -> String {
    if boards.is_empty() {
        return "none".to_owned();
    }
    boards
        .iter()
        .map(|board| {
            format!(
                "{} ({:?}, {})",
                board.id,
                plain_field(&board.title).to_string(),
                plain_field(&board.state)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn known_state(value: &str, noun: &str) -> Result<&'static str, Error> {
    match value {
        "open" => Ok("open"),
        "closed" => Ok("closed"),
        other => Err(Error::data(format!(
            "Forgejo returned unknown {noun} state {other:?}"
        ))),
    }
}

fn result(
    json: bool,
    action: &'static str,
    number: u64,
    board: u64,
    column: &ColumnResponse,
) -> Result<Outcome, Error> {
    if json {
        Outcome::json(&ResultRecord {
            kind: "result",
            action,
            ok: true,
            number,
            board,
            column: &column.title,
            column_id: column.id,
        })
    } else {
        Ok(Outcome::text(format!(
            "{action}\t{number}\t{}\tok\n",
            plain_field(&column.title)
        )))
    }
}

fn format_error(error: std::fmt::Error) -> Error {
    Error::data(format!("could not format board output: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        BoardRecord, BoardResponse, CardResponse, ColumnResponse, TitleMatch, match_title,
    };

    const TITLES: &[&str] = &["Backlog", "To Do", "In Progress", "Blocked", "Done"];

    fn position(titles: &[&str], wanted: &str) -> Option<usize> {
        match match_title(titles, wanted) {
            TitleMatch::One(index) => Some(index),
            TitleMatch::None | TitleMatch::Many => None,
        }
    }

    #[test]
    fn column_titles_match_ignoring_case_but_never_by_prefix() {
        assert_eq!(position(TITLES, "In Progress"), Some(2));
        assert_eq!(position(TITLES, "in progress"), Some(2));
        assert_eq!(position(TITLES, "DONE"), Some(4));
        for wanted in ["", "Do", "In  Progress", "Progress", "To Do "] {
            assert!(matches!(match_title(TITLES, wanted), TitleMatch::None));
        }
    }

    #[test]
    fn an_exact_title_wins_and_other_duplicates_are_ambiguous() {
        let titles = ["done", "Done", "DONE"];
        assert_eq!(position(&titles, "Done"), Some(1));
        assert!(matches!(match_title(&titles, "dOnE"), TitleMatch::Many));
        assert!(matches!(
            match_title(&["Done", "Done"], "Done"),
            TitleMatch::Many
        ));
    }

    #[test]
    fn gitea_board_fixtures_decode() {
        let boards: Vec<BoardResponse> = serde_json::from_str(include_str!(
            "../../tests/fixtures/gitea-28.2.0/boards.json"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
        let record = BoardRecord::try_from(
            boards
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("fixture has a board")),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!((record.id, record.state), (7, "open"));
        assert_eq!(record.title, "Development");

        let columns: Vec<ColumnResponse> = serde_json::from_str(include_str!(
            "../../tests/fixtures/gitea-28.2.0/board-columns.json"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
        let order: Vec<_> = columns.iter().map(|column| column.title.as_str()).collect();
        assert_eq!(order, super::tests::TITLES);
        assert!(columns[0].default);
        // Board order is response order: "Blocked" has the newest, largest ID.
        assert_eq!(columns[3].id, 31);

        let cards: Vec<CardResponse> = serde_json::from_str(include_str!(
            "../../tests/fixtures/gitea-28.2.0/board-cards.json"
        ))
        .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!((cards[0].number, cards[0].id), (16, 756));
        assert!(cards[0].pull_request.is_none());
    }
}
