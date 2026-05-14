use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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

pub(super) fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

pub(super) fn open_vim_view(view: &VimView) -> Result<()> {
    let editor = resolve_vim_editor()?;
    let dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let content_path = dir.join(format!("sivtr-view-{}-{nonce}.txt", std::process::id()));
    let vimrc_path = dir.join(format!("sivtr-view-{}-{nonce}.vim", std::process::id()));
    let blocks_path = dir.join(format!(
        "sivtr-view-{}-{nonce}.blocks.json",
        std::process::id()
    ));

    std::fs::write(&content_path, &view.raw).context("Failed to write Vim view file")?;
    let blocks_json =
        serde_json::to_string(&view.blocks).context("Failed to encode Vim block data")?;
    std::fs::write(&blocks_path, blocks_json).context("Failed to write Vim block data")?;
    write_vimrc(&vimrc_path, &blocks_path)?;

    let parts: Vec<&str> = editor.split_whitespace().collect();
    let (program, extra_args) = parts
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("Empty Vim editor command"))?;

    let status = Command::new(program)
        .args(extra_args)
        .arg("-u")
        .arg(&vimrc_path)
        .arg("-n")
        .arg("-R")
        .arg(&content_path)
        .status()
        .with_context(|| format!("Failed to launch Vim editor `{editor}`"))?;

    let _ = std::fs::remove_file(&content_path);
    let _ = std::fs::remove_file(&vimrc_path);
    let _ = std::fs::remove_file(&blocks_path);

    if !status.success() {
        anyhow::bail!("Vim editor `{editor}` exited with {status}");
    }
    Ok(())
}

fn resolve_vim_editor() -> Result<String> {
    let config = sivtr_core::config::SivtrConfig::load().unwrap_or_default();
    if is_vim_command(&config.editor.command) {
        return Ok(config.editor.command);
    }

    for candidate in ["nvim", "vim", "vi"] {
        if command_exists(candidate) {
            return Ok(candidate.to_string());
        }
    }

    anyhow::bail!("No Vim-compatible editor found. Set `editor.command` to nvim/vim/vi.")
}

pub(super) fn is_vim_command(command: &str) -> bool {
    let Some(program) = command.split_whitespace().next() else {
        return false;
    };
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
