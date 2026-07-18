use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

const TEMP_CLEANUP_ATTEMPTS: usize = 3;
const TEMP_CLEANUP_RETRY_DELAY: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub(super) struct VimView {
    pub(super) raw: String,
    pub(super) blocks: Vec<VimBlock>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct VimBlock {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) input_start: usize,
    pub(super) input_end: usize,
    pub(super) output_start: usize,
    pub(super) output_end: usize,
    pub(super) block_text: String,
    pub(super) input_text: String,
    pub(super) output_text: String,
    pub(super) command_text: String,
}

pub(super) fn open_vim_view(view: &VimView) -> Result<()> {
    let editor = resolve_vim_editor()?;
    // A securely randomized directory prevents predictable-name collisions and is deleted on
    // every return path, including write, spawn, and non-zero-exit failures.
    let temp_dir = tempfile::Builder::new()
        .prefix("sivtr-view-")
        .tempdir()
        .context("Failed to create temporary Vim view directory")?;
    let content_path = temp_dir.path().join("content.txt");
    let vimrc_path = temp_dir.path().join("view.vim");
    let blocks_path = temp_dir.path().join("blocks.json");

    let operation = (|| -> Result<()> {
        std::fs::write(&content_path, &view.raw).context("Failed to write Vim view file")?;
        let blocks_json =
            serde_json::to_string(&view.blocks).context("Failed to encode Vim block data")?;
        std::fs::write(&blocks_path, blocks_json).context("Failed to write Vim block data")?;
        write_vimrc(&vimrc_path, &blocks_path)?;

        let (program, extra_args) = sivtr_core::export::editor::parse_editor_command(&editor)?;

        let status = Command::new(&program)
            .args(&extra_args)
            .arg("-u")
            .arg(&vimrc_path)
            .arg("-n")
            .arg("-R")
            .arg(&content_path)
            .status()
            .with_context(|| format!("Failed to launch Vim editor `{editor}`"))?;

        if !status.success() {
            anyhow::bail!("Vim editor `{editor}` exited with {status}");
        }
        Ok(())
    })();
    let cleanup = cleanup_temp_dir(temp_dir);

    finish_vim_view(operation, cleanup)
}

fn cleanup_temp_dir(temp_dir: tempfile::TempDir) -> Result<()> {
    let path = temp_dir.path().to_path_buf();
    let cleanup = remove_dir_all_with_retry(
        &path,
        TEMP_CLEANUP_ATTEMPTS,
        TEMP_CLEANUP_RETRY_DELAY,
        |path| std::fs::remove_dir_all(path),
    );
    // Keep TempDir's destructor as a final best-effort attempt if all reported retries fail.
    drop(temp_dir);
    if cleanup.is_err() && matches!(path.try_exists(), Ok(false)) {
        return Ok(());
    }
    cleanup.with_context(|| {
        format!(
            "Failed to securely remove temporary Vim view files from {}",
            path.display()
        )
    })
}

fn remove_dir_all_with_retry(
    path: &Path,
    attempts: usize,
    retry_delay: Duration,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    debug_assert!(attempts > 0);
    let mut last_error = None;

    for attempt in 0..attempts {
        match remove(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => last_error = Some(error),
        }

        if attempt + 1 < attempts {
            std::thread::sleep(retry_delay);
        }
    }

    Err(last_error.expect("cleanup attempts must be greater than zero"))
}

fn finish_vim_view(operation: Result<()>, cleanup: Result<()>) -> Result<()> {
    match (operation, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{error:#}; additionally failed to remove temporary Vim view files: {cleanup_error:#}"
        )),
    }
}

fn resolve_vim_editor() -> Result<String> {
    let config = sivtr_core::config::SivtrConfig::load().unwrap_or_default();
    if let Some(editor) = resolve_configured_vim_editor(&config.editor.command)? {
        return Ok(editor);
    }

    for candidate in ["nvim", "vim", "vi"] {
        if command_exists(candidate) {
            return Ok(candidate.to_string());
        }
    }

    anyhow::bail!("No Vim-compatible editor found. Set `editor.command` to nvim/vim/vi.")
}

fn resolve_configured_vim_editor(command: &str) -> Result<Option<String>> {
    if command.is_empty() {
        return Ok(None);
    }

    let (program, _) = sivtr_core::export::editor::parse_editor_command(command)?;
    Ok(is_vim_program(&program).then(|| command.to_string()))
}

pub(super) fn is_vim_command(command: &str) -> bool {
    let Ok((program, _)) = sivtr_core::export::editor::parse_editor_command(command) else {
        return false;
    };
    is_vim_program(&program)
}

fn is_vim_program(program: &str) -> bool {
    let name = std::path::Path::new(program)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .to_lowercase();
    name == "vi" || name.contains("vim")
}

pub(super) fn vim_single_quote(value: &str) -> String {
    value.replace('\'', "''")
}

fn command_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        Command::new("where")
            .arg(name)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("which")
            .arg(name)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}

fn write_vimrc(path: &std::path::Path, blocks_path: &std::path::Path) -> Result<()> {
    let mut file = std::fs::File::create(path).context("Failed to create temporary Vim config")?;
    let blocks_path = vim_single_quote(&blocks_path.to_string_lossy());
    let script = format!(
        r#"
set nocompatible
set nomodeline
set readonly
set nomodifiable
set nomodified
set number
set nowrap
set nofoldenable
let s:sivtr_blocks = json_decode(join(readfile('{blocks_path}'), "\n"))

function! s:SivtrCurrentBlockIndex() abort
  let l:line = line('.')
  let l:previous = -1
  for l:i in range(0, len(s:sivtr_blocks) - 1)
    let l:block = s:sivtr_blocks[l:i]
    if l:line >= l:block.start && l:line <= l:block.end
      return l:i
    endif
    if l:block.start <= l:line
      let l:previous = l:i
    endif
  endfor
  return l:previous >= 0 ? l:previous : 0
endfunction

function! s:SivtrCurrentBlock() abort
  if empty(s:sivtr_blocks)
    echohl ErrorMsg | echo 'sivtr: no blocks' | echohl None
    return {{}}
  endif
  return s:sivtr_blocks[s:SivtrCurrentBlockIndex()]
endfunction

function! s:SivtrJump(delta) abort
  if empty(s:sivtr_blocks)
    echohl ErrorMsg | echo 'sivtr: no blocks' | echohl None
    return
  endif
  let l:idx = s:SivtrCurrentBlockIndex() + a:delta
  let l:idx = max([0, min([l:idx, len(s:sivtr_blocks) - 1])])
  call cursor(s:sivtr_blocks[l:idx].start, 1)
  normal! zz
endfunction

function! s:SivtrCopy(kind) abort
  let l:block = s:SivtrCurrentBlock()
  if empty(l:block)
    return
  endif
  let l:key = a:kind . '_text'
  let l:text = get(l:block, l:key, '')
  if empty(l:text)
    echohl ErrorMsg | echo 'sivtr: current block has no ' . a:kind . ' content' | echohl None
    return
  endif
  call setreg('"', l:text)
  try | call setreg('+', l:text) | catch | endtry
  try | call setreg('*', l:text) | catch | endtry
  echo 'sivtr: copied current ' . a:kind
endfunction

function! s:SivtrSelect(kind) abort
  let l:block = s:SivtrCurrentBlock()
  if empty(l:block)
    return
  endif
  if a:kind ==# 'block'
    let [l:start, l:end] = [l:block.start, l:block.end]
  elseif a:kind ==# 'input'
    let [l:start, l:end] = [l:block.input_start, l:block.input_end]
  else
    let [l:start, l:end] = [l:block.output_start, l:block.output_end]
  endif
  if l:start <= 0 || l:end <= 0
    echohl ErrorMsg | echo 'sivtr: current block has no ' . a:kind . ' range' | echohl None
    return
  endif
  call cursor(l:start, 1)
  normal! V
  call cursor(l:end, 1)
endfunction

nnoremap <silent> p :qa!<CR>
nnoremap <silent> q :qa!<CR>
nnoremap <silent> <Esc> :qa!<CR>
nnoremap <silent> [[ :call <SID>SivtrJump(-1)<CR>
nnoremap <silent> ]] :call <SID>SivtrJump(1)<CR>
nnoremap <silent> myy :call <SID>SivtrCopy('block')<CR>
nnoremap <silent> myi :call <SID>SivtrCopy('input')<CR>
nnoremap <silent> myo :call <SID>SivtrCopy('output')<CR>
nnoremap <silent> myc :call <SID>SivtrCopy('command')<CR>
nnoremap <silent> mvv :call <SID>SivtrSelect('block')<CR>
nnoremap <silent> mvi :call <SID>SivtrSelect('input')<CR>
nnoremap <silent> mvo :call <SID>SivtrSelect('output')<CR>
autocmd VimEnter * echo "sivtr: [[/]] jump blocks, myy/myi/myo/myc copy, mvv/mvi/mvo select, p returns to picker"
"#
    );
    file.write_all(script.as_bytes())
        .context("Failed to write temporary Vim config")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        finish_vim_view, is_vim_command, remove_dir_all_with_retry, resolve_configured_vim_editor,
    };
    use std::path::Path;
    use std::time::Duration;

    #[test]
    fn detects_quoted_windows_vim_path() {
        assert!(is_vim_command(
            r#""C:\Program Files\Neovim\bin\nvim.exe" --clean"#
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn rejects_invalid_configured_vim_command_instead_of_falling_back() {
        let error = resolve_configured_vim_editor("nvim --cmd 'set nowrap")
            .unwrap_err()
            .to_string();

        assert!(error.contains("Invalid editor command quoting"));
    }

    #[test]
    fn valid_non_vim_command_can_still_fall_back() {
        assert_eq!(resolve_configured_vim_editor("code --wait").unwrap(), None);
    }

    #[test]
    fn retries_temporary_directory_cleanup_before_succeeding() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().to_path_buf();
        let mut calls = 0;

        remove_dir_all_with_retry(&path, 3, Duration::ZERO, |path| {
            calls += 1;
            if calls < 3 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "temporary lock",
                ));
            }
            std::fs::remove_dir_all(path)
        })
        .unwrap();

        assert_eq!(calls, 3);
        assert!(!path.exists());
    }

    #[test]
    fn temporary_directory_cleanup_retries_are_bounded() {
        let mut calls = 0;
        let error = remove_dir_all_with_retry(Path::new("unused"), 3, Duration::ZERO, |_| {
            calls += 1;
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "still locked",
            ))
        })
        .unwrap_err();

        assert_eq!(calls, 3);
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn reports_both_editor_and_sensitive_temp_cleanup_failures() {
        let error = finish_vim_view(
            Err(anyhow::anyhow!("editor failed")),
            Err(anyhow::anyhow!("cleanup failed")),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("editor failed"));
        assert!(error.contains("cleanup failed"));
    }
}
