Run `sivtr doctor` to check the environment, then investigate any failing checks.

Steps:
1. Run `sivtr doctor` and capture output
2. For each failing check:
   - **config file missing**: run `sivtr config init`
   - **shell hooks not installed**: run `sivtr init bash` (or zsh/pwsh/nushell depending on shell)
   - **session log dir missing**: note that it's created after hook install + terminal restart
   - **provider sessions empty**: this is normal if no AI tools were used in this workspace
   - **clipboard unavailable**: check if xclip/wl-copy/pbcopy is installed
3. After fixes, run `sivtr doctor` again to confirm all checks pass
