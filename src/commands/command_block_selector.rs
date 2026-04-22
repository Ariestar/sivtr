use anyhow::{Context, Result};

#[allow(clippy::enum_variant_names)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandSelection {
    RecentSingle(usize),
    RecentRange { newer: usize, older: usize },
    RecentExplicit(Vec<usize>),
}

pub fn parse_selector(value: &str) -> Result<CommandSelection> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("Empty selector. Use `N`, `A..B`, or `--pick`.");
    }

    if let Some((a, b)) = value.split_once("..") {
        let a = parse_positive(a)?;
        let b = parse_positive(b)?;
        let (newer, older) = if a <= b { (a, b) } else { (b, a) };
        return Ok(CommandSelection::RecentRange { newer, older });
    }

    Ok(CommandSelection::RecentSingle(parse_positive(value)?))
}

pub fn resolve_selector(selection: CommandSelection, total: usize) -> Result<Vec<usize>> {
    match selection {
        CommandSelection::RecentSingle(recent) => {
            if recent == 0 {
                anyhow::bail!("Selector values are 1-based. Use `1` for the last command.");
            }
            if recent > total {
                anyhow::bail!(
                    "Only {total} command(s) recorded. Try a smaller selector or `--pick`."
                );
            }
            Ok(vec![total - recent])
        }
        CommandSelection::RecentRange { newer, older } => {
            if newer == 0 || older == 0 {
                anyhow::bail!("Range selectors are 1-based. Example: `2..5`.");
            }
            if older > total {
                anyhow::bail!("Only {total} command(s) recorded. Try a smaller range or `--pick`.");
            }
            let start = total - older;
            let end = total - newer;
            Ok((start..=end).collect())
        }
        CommandSelection::RecentExplicit(selected) => {
            if selected.is_empty() {
                anyhow::bail!("No command blocks selected.");
            }

            let mut indices = Vec::with_capacity(selected.len());
            for recent in selected {
                if recent == 0 {
                    anyhow::bail!("Selector values are 1-based. Use `1` for the last command.");
                }
                if recent > total {
                    anyhow::bail!(
                        "Only {total} command(s) recorded. Try a smaller selector or `--pick`."
                    );
                }
                indices.push(total - recent);
            }

            indices.sort_unstable();
            indices.dedup();
            Ok(indices)
        }
    }
}

fn parse_positive(value: &str) -> Result<usize> {
    let n = value
        .parse::<usize>()
        .with_context(|| format!("Invalid selector `{value}`. Use `N`, `A..B`, or `--pick`."))?;
    if n == 0 {
        anyhow::bail!("Selector values are 1-based. Use `1` for the last command.");
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::{parse_selector, resolve_selector, CommandSelection};

    #[test]
    fn parses_selection_count() {
        assert_eq!(
            parse_selector("3").unwrap(),
            CommandSelection::RecentSingle(3)
        );
    }

    #[test]
    fn resolves_single_selection_as_exact_recent_command() {
        assert_eq!(
            resolve_selector(CommandSelection::RecentSingle(3), 10).unwrap(),
            vec![7]
        );
    }

    #[test]
    fn parses_selection_range() {
        assert_eq!(
            parse_selector("5..2").unwrap(),
            CommandSelection::RecentRange { newer: 2, older: 5 }
        );
    }

    #[test]
    fn resolves_selection_range() {
        assert_eq!(
            resolve_selector(CommandSelection::RecentRange { newer: 2, older: 5 }, 10).unwrap(),
            vec![5, 6, 7, 8]
        );
    }

    #[test]
    fn resolves_explicit_selection_as_disjoint_commands() {
        assert_eq!(
            resolve_selector(CommandSelection::RecentExplicit(vec![1, 4, 7]), 10).unwrap(),
            vec![3, 6, 9]
        );
    }
}
