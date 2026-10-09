@echo off
setlocal enabledelayedexpansion
cd /d "%~dp0"

rem Run it bare for a menu, or pass the action as an argument. Exit code is
rem non-zero if anything failed, so it can be chained.
rem
rem **TelegramExporter's save.bat, with this repo's checks in it.** Same menu,
rem same order, same git flow -- the exporter's `parity` legs become this repo's
rem `baseline`, which is the check here that cannot be faked. Change the shape
rem there first and bring it across, or the two scripts drift into two habits.

set REPO=https://github.com/KostaJovanovic/TelegramAnalyser.git
set EXENAME=TelegramAnalyser.exe

rem The baseline's corpora. They live on removable drives, so every step that
rem uses one degrades to a skip when the drive is not mounted rather than
rem failing the run.
set "UAEXPORT=N:\telegram export\UA KOLAB TELEGRAM"

rem **The KRGM folder is not spelled out, and that is deliberate.** Its name is
rem "KRGM TRECI 2026.06.09" with a C-acute, and a .bat is read in the console's
rem OEM codepage rather than as UTF-8 -- so the literal would arrive mangled on
rem any machine whose console is not cp1250, and `if exist` would then report a
rem mounted drive as missing. `for /d` gets the name from the filesystem, in
rem whatever encoding the shell is already using, and hands it straight back out
rem to cargo through the same layer.
set "KRGMEXPORT="
for /d %%d in ("J:\temp pureraw\KRGM*") do set "KRGMEXPORT=%%~fd"
rem Square brackets, not round: this value is echoed inside `if (...)` blocks,
rem where a `)` closes the block early and the line printed "(not mounted".
if not defined KRGMEXPORT set "KRGMEXPORT=J:\temp pureraw\KRGM [not mounted]"

rem This machine owns the exports, so a `cargo test` here that finds no corpus
rem has not "skipped the legs" -- it has a broken setup, and libtest throws away
rem the eprintln that would have said so. With this set, the corpus tests panic
rem instead. `set TGA_REQUIRE_CORPUS=0` before calling opts back out; plain
rem `cargo test --all` is unaffected, which is what keeps a fresh clone working.
if not defined TGA_REQUIRE_CORPUS set "TGA_REQUIRE_CORPUS=1"

rem Two clocks. T0 covers the whole run and is reported at the end; TS is
rem restarted per step, so a slow run says *which* step was slow rather than
rem leaving you watching a crate counter and guessing.
call :clock T0

set SAVE_ERROR=0
set COMMIT_ONLY=0
set FORCE_MODE=0
set DO_BUILD=0
set ACTION=%~1

rem --force pushes with --force-with-lease, not a bare --force: it still
rem overwrites origin/%BRANCH%, but refuses if origin moved since the last
rem fetch, so it cannot silently clobber a push you have not seen yet.
rem --force-if-includes rides with it everywhere, because the lease alone is
rem weaker than it reads. --force-with-lease compares against the *local*
rem remote-tracking ref, refs/remotes/origin/%BRANCH% -- not against the remote.
rem Anything that fetches in the background (an IDE, a `git fetch` in another
rem window, a periodic prefetch) advances that ref without showing anybody the
rem commits it just recorded, and the lease is then satisfied by a state the
rem user has never actually looked at -- exactly the clobber the lease was
rem chosen to prevent. --force-if-includes additionally requires that whatever
rem is being overwritten is already an ancestor of what is being pushed, i.e.
rem that the local work was built on top of it, so a fetch that nobody read
rem cannot license the overwrite. It needs git 2.30 or newer; on an older git
rem the push is rejected outright rather than silently unprotected.
rem Quoted `set "NAME=value"` throughout, and not for style: `set DO_BUILD=1 &`
rem assigns the string "1 " -- cmd takes everything between the = and the & as
rem the value, trailing space included. The later `if "%DO_BUILD%"=="1"` then
rem compares "1 " against "1" and never matches, so `save.bat release` ran the
rem tests, committed, pushed, and silently skipped the build it was asked for.
if /i "%ACTION%"=="--force"   (set "FORCE_MODE=1" & set "ACTION=save")
if /i "%ACTION%"=="--commit"  (set "COMMIT_ONLY=1" & set "ACTION=save")
if /i "%ACTION%"=="--no-push" (set "COMMIT_ONLY=1" & set "ACTION=save")
if /i "%ACTION%"=="commit"    (set "COMMIT_ONLY=1" & set "ACTION=save")
if /i "%ACTION%"=="save"     goto save
if /i "%ACTION%"=="release"  (set "DO_BUILD=1" & goto save)
if /i "%ACTION%"=="push"     goto push
if /i "%ACTION%"=="pull"     goto pull
if /i "%ACTION%"=="build"    goto build
if /i "%ACTION%"=="exe"      goto build
if /i "%ACTION%"=="test"     goto test
if /i "%ACTION%"=="tests"    goto test
if /i "%ACTION%"=="baseline" goto baseline
if /i "%ACTION%"=="bless"    goto bless
if /i "%ACTION%"=="report"   goto report
if /i "%ACTION%"=="run"      goto runwindow
if /i "%ACTION%"=="window"   goto runwindow
if /i "%ACTION%"=="clean"    goto clean

:menu
echo.
echo === telegram analyser (rust) ===
echo.
echo   1  save     test + commit + push
echo   2  commit   test + commit, no push
echo   3  push     push current branch
echo   4  pull     pull current branch
echo   5  build    cargo build --release
echo   6  release  test + commit + push + build
echo   7  test     fmt + clippy + every suite
echo   8  baseline diff both archives against the last recording
echo   9  bless    re-record the committed golden
echo   10 report   write report.html for an export
echo   11 run      open the window
echo   12 clean    report the build cache, and empty it
echo   13 quit
echo.
echo   save.bat --force          runs save and force-with-lease pushes without asking
echo   save.bat baseline record  re-records the baseline, after reading the diff
echo.
set /p CHOICE=select [1-13]:
if "%CHOICE%"=="1" goto save
if "%CHOICE%"=="2" (set "COMMIT_ONLY=1" & goto save)
if "%CHOICE%"=="3" goto push
if "%CHOICE%"=="4" goto pull
if "%CHOICE%"=="5" goto build
if "%CHOICE%"=="6" (set "DO_BUILD=1" & goto save)
if "%CHOICE%"=="7" goto test
if "%CHOICE%"=="8" goto baseline
if "%CHOICE%"=="9" goto bless
if "%CHOICE%"=="10" goto report
if "%CHOICE%"=="11" goto runwindow
if "%CHOICE%"=="12" goto clean
if "%CHOICE%"=="13" exit /b 0
echo [err]  invalid choice
goto menu


rem ---------------------------------------------------------------------------
:save
echo.
echo === git: save ===
echo.

call :checkcargo
if errorlevel 1 goto end

for /f %%i in ('git rev-list --count HEAD 2^>nul') do set COMMIT_COUNT=%%i
if not defined COMMIT_COUNT set COMMIT_COUNT=0
set /a NEXT_COUNT=%COMMIT_COUNT%+1

rem Version label: commits crowned as major releases go in RELEASES
rem (ascending) and each one reads X.0 and restarts the
rem minor counter. Nothing has been crowned yet, so this reads v0.NN.
set RELEASES=
for /f %%v in ('powershell -NoProfile -Command "$n=%NEXT_COUNT%; $major=0; $base=0; foreach($r in @(%RELEASES%)){ if($n -ge $r){ $major++; $base=$r } else { break } }; if($major -eq 0){ '0.{0:D2}' -f $n } elseif(($n-$base) -eq 0){ '{0}.0' -f $major } else { '{0}.{1:D2}' -f $major,($n-$base) }"') do set VERLABEL=%%v
echo bump: v%VERLABEL% (commit %NEXT_COUNT%)

rem Formatting and lints first: they are the fastest to fail and the cheapest to
rem fix, and a suite run before them just delays the same answer.
echo.
echo [chk]  cargo fmt
call :clock TS
cargo fmt --all --check
if errorlevel 1 goto testsfailed
call :since TS "cargo fmt"

echo.
echo [chk]  cargo clippy
call :clock TS
cargo clippy --all-targets --all-features -- -D warnings
if errorlevel 1 goto testsfailed
call :since TS "cargo clippy"

echo.
echo [chk]  cargo test   (TGA_REQUIRE_CORPUS=%TGA_REQUIRE_CORPUS%)
call :clock TS
rem --nocapture, unlike the exporter's: a corpus skip here is an eprintln, and
rem libtest discards stdout and stderr for a passing test. A skip nobody sees is
rem how a suite reports as coverage while having compared nothing.
cargo test --all -- --nocapture
if errorlevel 1 goto testsfailed
call :since TS "cargo test"

rem The output is verified, not asserted: the whole report and the whole stats
rem dump, byte for byte, against the last recording. Skipped, loudly, when
rem nothing is recorded or the drives are not here. A red leg lands in
rem :testsfailed with the other checks, which still offers "commit anyway";
rem what it does not do is decide that for you.
call :runbaseline
if errorlevel 1 goto testsfailed

echo.
echo [git]  stage
git add .
git status

rem Nothing staged is not a failure: the work may already be committed and only
rem waiting to be pushed. Asked before the message prompt, so a clean tree does
rem not ask for a message it will not use. A commit that fails for any other
rem reason - a hook rejecting it - still stops here.
git diff --cached --quiet
if not errorlevel 1 (
  echo.
  echo [git]  nothing new to commit - the earlier commits are what goes out
  goto pushstep
)

echo.
set /p MSG=commit message [v%VERLABEL%]:
if "%MSG%"=="" set MSG=v%VERLABEL%

git commit -m "%MSG%"
if errorlevel 1 (
  echo.
  echo [err]  git commit failed - a hook rejected it
  set SAVE_ERROR=1
  goto end
)
echo.
echo [git]  committed v%VERLABEL%

:pushstep
if "%COMMIT_ONLY%"=="1" goto afterpush

call :branchname
call :hasremote
if errorlevel 1 goto afterpush

if "%FORCE_MODE%"=="1" goto forcepush

echo.
set /p DOPUSH=push to origin/%BRANCH%? (y/n):
if /i not "%DOPUSH%"=="y" goto pushskipped

git push -u origin %BRANCH%
if not errorlevel 1 goto pushed

echo.
echo [warn] push rejected - the remote is probably ahead of local
echo.
set /p FETCH=pull + merge remote first? (y/n):
if /i "%FETCH%"=="y" goto fetch

set /p FORCE=force-with-lease push instead? overwrites the remote unless it carries work you have not built on. (y/n):
if /i "%FORCE%"=="y" goto forcepush

echo [git]  skipped - nothing pushed
set SAVE_ERROR=1
goto afterpush

:fetch
git pull origin %BRANCH%
if errorlevel 1 set SAVE_ERROR=1
echo.
echo [git]  pulled - resolve any conflicts, then re-run
goto afterpush

:forcepush
call :branchname
git push -u origin %BRANCH% --force-with-lease --force-if-includes
if errorlevel 1 set SAVE_ERROR=1
echo.
echo [git]  force-with-lease pushed origin/%BRANCH%
goto afterpush

:pushed
echo.
echo [git]  pushed origin/%BRANCH%
goto afterpush

:pushskipped
echo.
echo [git]  push skipped

:afterpush
if "%DO_BUILD%"=="1" goto build
goto end

:testsfailed
echo.
echo [err]  a check failed - see the output above
echo.
set /p ANYWAY=commit anyway? (y/n):
if /i "%ANYWAY%"=="y" goto stageanyway
set SAVE_ERROR=1
goto end

:stageanyway
echo [warn] committing with failing checks
echo.
echo [git]  stage
git add .
git status
echo.
set /p MSG=commit message [v%VERLABEL%]:
if "%MSG%"=="" set MSG=v%VERLABEL%
git commit -m "%MSG%"
if errorlevel 1 set SAVE_ERROR=1
goto afterpush


rem ---------------------------------------------------------------------------
:build
echo.
echo === build: exe ===
echo.

call :checkcargo
if errorlevel 1 goto end

rem A running instance holds both the linker output and dist\%EXENAME% open,
rem so the build fails and then the copy does. Killing it first is not optional.
rem taskkill exits non-zero when nothing was running, which is the normal case.
echo [exe]  stop any running instance
taskkill /F /IM %EXENAME% >nul 2>&1
if errorlevel 1 (echo        none running) else (echo        stopped)

echo [exe]  cargo build --release
call :clock TS
cargo build --release -p tga-app
if errorlevel 1 (
  echo.
  echo [err]  build failed
  set SAVE_ERROR=1
  goto end
)
call :since TS "release build"

if not exist "target\release\%EXENAME%" (
  echo [err]  build reported success but target\release\%EXENAME% is missing
  set SAVE_ERROR=1
  goto end
)

rem The exe ships from dist\, not from target\. cargo clean empties target\,
rem and `save.bat clean` is a menu entry: an exe you actually use should not
rem live somewhere a maintenance command deletes. dist\ is a folder you can
rem copy anywhere and it still works -- nothing is written beside this exe.
if not exist "dist" mkdir dist
copy /y "target\release\%EXENAME%" "dist\%EXENAME%" >nul
if errorlevel 1 (
  echo [err]  could not copy the exe into dist\ - is it still running?
  set SAVE_ERROR=1
  goto end
)

for /f %%s in ('powershell -NoProfile -Command "'{0:N1}' -f ((Get-Item 'dist\%EXENAME%').Length/1MB)"') do set EXESIZE=%%s
echo.
echo [ok]   dist\%EXENAME%  %EXESIZE% MB

rem The cache, reported where somebody will actually read it. cargo never
rem garbage-collects target\, and in the exporter's tree it reached 45 GB
rem before a single line of any script mentioned it. A few seconds of scan on
rem the back of a release build is a fair price for that never happening
rem again, and `save.bat clean` is what to do about the number.
call :dirsize "target" "target        "
goto end


rem ---------------------------------------------------------------------------
:test
echo.
echo === test: fmt, clippy, every suite ===
echo.
call :checkcargo
if errorlevel 1 goto end
call :clock TS
cargo fmt --all --check
if errorlevel 1 set SAVE_ERROR=1
call :since TS "cargo fmt"
call :clock TS
cargo clippy --all-targets --all-features -- -D warnings
if errorlevel 1 set SAVE_ERROR=1
call :since TS "cargo clippy"
call :clock TS
cargo test --all -- --nocapture
if errorlevel 1 set SAVE_ERROR=1
call :since TS "cargo test"
goto end


rem ---------------------------------------------------------------------------
rem What this program wrote before, against what it writes now.
rem
rem **This is what replaced the Python oracle.** That harness could only ever
rem answer "does this still match the program it was ported from", which stops
rem being a useful question the moment the two are meant to differ. This asks
rem the question that survives: did anything I just changed alter one byte of
rem the report? A difference is a mistake rather than a judgement call, unless
rem the change was meant to alter the output -- and then it is
rem `save.bat baseline record`, after reading the diff.
rem
rem Both files are compared byte for byte, the stats dump included. That is only
rem possible because the dump's layout is held still on purpose -- see the
rem serde note on `Count` in tga-stats.
:baseline
echo.
if /i "%~2"=="record" goto baserecord
echo === test: baseline ===
echo.
call :checkcargo
if errorlevel 1 goto end
call :runbaseline
if errorlevel 1 set SAVE_ERROR=1
goto end

:baserecord
echo === baseline: record what the report looks like now ===
call :checkcargo
if errorlevel 1 goto end
cargo build -q --release -p tga-app
if errorlevel 1 (set SAVE_ERROR=1 & goto end)
if not exist "baseline" mkdir "baseline"
rem Three legs, not two. Neither export has an events file beside it, so the
rem first two leave the entire annotation layer -- the rail, the markers, the
rem cards, the coverage band, the "matches nobody" list -- unrecorded. The third
rem points at the 42 notes that were written by hand about the KRGM archive.
call :onerecord "ua-kolab" "%UAEXPORT%"
call :onerecord "krgm" "%KRGMEXPORT%"
call :onerecord "krgm-notes" "%KRGMEXPORT%" "%KRGMEXPORT%\_timeline\events\general.json"
echo.
echo [note] baseline\ is gitignored. It is whole reports on real archives,
echo        which is other people's conversation.
goto end


rem ---------------------------------------------------------------------------
:bless
echo.
echo === test: bless the golden ===
echo.
call :checkcargo
if errorlevel 1 goto end
set "TGA_BLESS=1"
cargo test -p tga-report --test golden -- --nocapture
if errorlevel 1 set SAVE_ERROR=1
set "TGA_BLESS="
echo.
echo [note] read the diff before committing it. A golden updated without being
echo        read is a golden that records whatever the bug did.
goto end


rem ---------------------------------------------------------------------------
rem Write report.html beside an export: a folder, a database, or -- with
rem --refresh -- every database the exporter syncs into one folder.
:report
call :askfolder "%~2"
if errorlevel 1 goto end
echo.
echo === report: %FOLDER% ===
echo.
call :checkcargo
if errorlevel 1 goto end
call :clock TS
cargo run -q --release -p tga-app --bin TelegramAnalyser -- "%FOLDER%" --quiet
if errorlevel 1 set SAVE_ERROR=1
call :since TS "report"
goto end


rem ---------------------------------------------------------------------------
:runwindow
echo.
echo === run: the window ===
echo.
rem **`cargo run`, not `start dist\...`.** `start` inside a batch that has its
rem output redirected does not detach -- cmd holds on and the action never
rem returns, so `save.bat run` would hang the terminal while looking like it had
rem detached. To run what `build` shipped, run it: `dist\TelegramAnalyser.exe`.
call :checkcargo
if errorlevel 1 goto end
cargo run -p tga-app --bin TelegramAnalyser
if errorlevel 1 set SAVE_ERROR=1
goto end


rem ---------------------------------------------------------------------------
rem target\ is a cache, and cargo never garbage-collects it: a dep bump, a
rem feature change or a toolchain move gives an artifact a new metadata hash and
rem the old one stays for good. In the exporter it reached **45 GB** unremarked
rem before anything measured it. The report is the point; the clean is just
rem what you do about it. Sizes are read once and printed even if you then pick
rem "nothing", because knowing the number is most of the fix.
:clean
echo.
echo === clean: the build cache ===
echo.
call :checkcargo
if errorlevel 1 goto end
call :dirsize "target\debug"   "target\debug  "
call :dirsize "target\release" "target\release"
echo.
echo   1  debug    cargo clean --profile dev
echo   2  release  cargo clean --release
echo   3  both     cargo clean
echo   4  nothing
echo.
set /p WHAT=remove [1-4]:
if "%WHAT%"=="1" goto cleandebug
if "%WHAT%"=="2" goto cleanrelease
if "%WHAT%"=="3" goto cleanboth
goto cleankept

:cleandebug
call :clock TS
cargo clean --profile dev
if errorlevel 1 set SAVE_ERROR=1
call :since TS "clean"
goto cleandone

:cleanrelease
call :clock TS
cargo clean --release
if errorlevel 1 set SAVE_ERROR=1
call :since TS "clean"
goto cleandone

:cleanboth
call :clock TS
cargo clean
if errorlevel 1 set SAVE_ERROR=1
call :since TS "clean"

:cleandone
rem Worth saying out loud: the next build is cold, with codegen-units = 1 and
rem thin LTO. dist\%EXENAME% is untouched -- it lives outside target\ for this
rem reason.
echo.
echo [ok]   cache emptied - the next build is a cold one
goto end

:cleankept
echo.
echo [ok]   nothing removed
goto end


rem ---------------------------------------------------------------------------
:push
echo.
echo === git: push ===
echo.
call :branchname
call :hasremote
if errorlevel 1 goto end
git push -u origin %BRANCH%
if not errorlevel 1 goto end
echo.
set /p FORCE=push failed. force-with-lease push? overwrites the remote unless it carries work you have not built on. (y/n):
if /i not "%FORCE%"=="y" goto pushgaveup
git push -u origin %BRANCH% --force-with-lease --force-if-includes
if errorlevel 1 set SAVE_ERROR=1
goto end

:pushgaveup
set SAVE_ERROR=1
goto end


rem ---------------------------------------------------------------------------
:pull
echo.
echo === git: pull ===
echo.
call :branchname
call :hasremote
if errorlevel 1 goto end
git pull origin %BRANCH%
if errorlevel 1 set SAVE_ERROR=1
goto end


rem ---------------------------------------------------------------------------
rem Helpers. Each returns via exit /b, so `if errorlevel 1` after the call reads
rem the helper's own result.

rem Whichever branch is checked out. This repo is on master, not main, and
rem hardcoding either one makes the script wrong the first time that changes.
:branchname
for /f %%b in ('git rev-parse --abbrev-ref HEAD 2^>nul') do set BRANCH=%%b
if not defined BRANCH set BRANCH=master
exit /b 0

rem Points at REPO. A checkout with no origin gets one rather than an error --
rem there is exactly one right answer here and making the user type it is only
rem a chance to type it wrong. An origin that is already set is left alone: it
rem may be a fork, or SSH where REPO is HTTPS, and silently repointing someone's
rem remote is not this script's business.
:hasremote
git remote get-url origin >nul 2>&1
if not errorlevel 1 exit /b 0
echo.
echo [git]  no 'origin' remote - adding %REPO%
git remote add origin %REPO%
if errorlevel 1 (
  echo [err]  could not add the remote
  set SAVE_ERROR=1
  exit /b 1
)
exit /b 0

rem Unix seconds via PowerShell rather than %TIME%, which is locale-formatted
rem and wraps at midnight -- a build started at 23:58 would report as negative.
:clock
for /f %%t in ('powershell -NoProfile -Command "[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()"') do set %~1=%%t
exit /b 0

rem :since <clock var> <label>  ->  [time] <label> 4m 12s
:since
set CLKVAL=!%~1!
if not defined CLKVAL exit /b 0
rem usebackq + backticks, so the PowerShell can use single quotes: the plain
rem for /f form wraps the command in single quotes itself and there is no way
rem to escape one inside it. Floor, not [int]: PowerShell rounds to nearest
rem on a cast, so 752s printed as 13m 32s.
for /f "usebackq delims=" %%e in (`powershell -NoProfile -Command "$s=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()-!CLKVAL!; if($s -ge 60){'{0}m {1:D2}s' -f [math]::Floor($s/60),($s%%60)}else{'{0}s' -f $s}"`) do set TOOK=%%e
echo [time] %~2 !TOOK!
exit /b 0

rem :dirsize <path relative to the repo> <label>
rem   ->  [size] target\debug   41.26 GB  62,326 files
rem
rem A missing directory prints "not built" rather than 0, because those are
rem different facts and only one of them means "already clean".
rem
rem %~dp0, not the relative path as given, so the answer does not depend on the
rem working directory of whatever cmd hands the command to.
rem
rem **No pipe in the PowerShell**, which is why the sum is a foreach and not
rem `Measure-Object`. `for /f` re-parses the backquoted string through a second
rem cmd, and a `^|` does not survive both passes intact: the pipeline never ran,
rem `$s` stayed null, and `'{0:N2}' -f $null` printed a perfectly formatted
rem **"0.00 GB, 0 files" for a 1.5 GB tree**. A size report that reads zero when
rem it cannot measure is worse than no report at all -- it says the problem is
rem solved. Semicolons only here; keep it that way.
:dirsize
set "DSPATH=%~dp0%~1"
if not exist "%DSPATH%" (
  echo [size] %~2 not built
  exit /b 0
)
for /f "usebackq delims=" %%z in (`powershell -NoProfile -Command "$f=Get-ChildItem -LiteralPath '%DSPATH%' -Recurse -Force -File -ErrorAction SilentlyContinue; $b=0; foreach($x in $f){$b+=$x.Length}; '{0,8:N2} GB  {1,7:N0} files' -f ($b/1GB),$f.Count"`) do echo [size] %~2 %%z
exit /b 0

:checkcargo
where cargo >nul 2>&1
if not errorlevel 1 exit /b 0
echo [err]  cargo not found on PATH - install Rust from https://rustup.rs
set SAVE_ERROR=1
exit /b 1

rem :askfolder <folder or empty>  ->  FOLDER
rem
rem Offers the two baseline corpora by number, because typing a path with a
rem Serbian character in it into a cmd prompt is its own adventure.
:askfolder
set "FOLDER=%~1"
if not "%FOLDER%"=="" goto :askfolder_check
echo.
echo   1  %UAEXPORT%
echo   2  %KRGMEXPORT%
echo   or paste a folder or a .sqlite
echo.
set /p FOLDER=export:
if "%FOLDER%"=="1" set "FOLDER=%UAEXPORT%"
if "%FOLDER%"=="2" set "FOLDER=%KRGMEXPORT%"
:askfolder_check
rem Explorer's "Copy as path" wraps in quotes and `set /p` keeps them.
set "FOLDER=%FOLDER:"=%"
if "%FOLDER%"=="" (
  echo [err]  no export given
  set SAVE_ERROR=1
  exit /b 1
)
if not exist "%FOLDER%" (
  echo [err]  not there: %FOLDER%
  set SAVE_ERROR=1
  exit /b 1
)
exit /b 0

rem Every recorded leg, compared. A *missing* recording or drive is non-fatal: a
rem machine without the archives is allowed to commit. A *differing* leg is not,
rem and it returns non-zero so :save sends it to :testsfailed -- the exporter's
rem parity used to print [warn] and return 0, which committed and pushed over a
rem red oracle while saying it was red.
:runbaseline
set "BASE_FAILED=0"
if not exist "baseline\*.report.html" (
  echo [skip] baseline - nothing recorded. Run: save.bat baseline record
  exit /b 0
)
echo.
echo [chk]  baseline
cargo build -q --release -p tga-app
if errorlevel 1 exit /b 1
call :onebase "ua-kolab" "%UAEXPORT%"
call :onebase "krgm" "%KRGMEXPORT%"
call :onebase "krgm-notes" "%KRGMEXPORT%" "%KRGMEXPORT%\_timeline\events\general.json"
exit /b !BASE_FAILED!

rem `--stamp` is not decoration. The stamp is the one value in the report that
rem comes from the clock, so it is pinned here to a date that is obviously not a
rem real one. Recording on Tuesday and comparing on Friday would otherwise
rem differ in the Notes line and nowhere else, which is the kind of failure that
rem teaches you to ignore the output. Both legs pass the same literal.
:onerecord
if not exist "%~2" (
  echo [skip] %~1: no export at %~2
  exit /b 0
)
echo.
echo [rec]  %~1
set "NOTESARG="
if not "%~3"=="" set NOTESARG=--notes "%~3"
call :clock TS
target\release\%EXENAME% "%~2" --quiet --stamp "1 January 2000" !NOTESARG! --out "baseline\%~1.report.html" --stats "baseline\%~1.stats.json"
if errorlevel 1 (set SAVE_ERROR=1 & exit /b 1)
call :since TS "%~1"
exit /b 0

:onebase
if not exist "%~2" (
  echo [skip] %~1: no export at %~2
  exit /b 0
)
if not exist "baseline\%~1.report.html" (
  echo [skip] %~1: never recorded
  exit /b 0
)
set "NOTESARG="
if not "%~3"=="" set NOTESARG=--notes "%~3"
call :clock TS
target\release\%EXENAME% "%~2" --quiet --stamp "1 January 2000" !NOTESARG! --out "baseline\_now-%~1.report.html" --stats "baseline\_now-%~1.stats.json"
if errorlevel 1 (set "BASE_FAILED=1" & exit /b 1)
call :samebytes "baseline\%~1.report.html" "baseline\_now-%~1.report.html" "%~1 report"
call :samebytes "baseline\%~1.stats.json"  "baseline\_now-%~1.stats.json"  "%~1 stats "
call :since TS "%~1"
exit /b 0

rem fc /b rather than a hash, because when it does differ the first differing
rem offset is the thing worth having and a hash cannot give it.
:samebytes
fc /b "%~1" "%~2" >nul 2>&1
if errorlevel 1 (
  echo [warn] %~3 differs from the baseline
  echo        fc /b "%~1" "%~2"
  set "BASE_FAILED=1"
  exit /b 1
)
echo [ok]   %~3 identical
exit /b 0


:end
call :since T0 "total"
echo.
rem The pause is for the double-click case, where the window would otherwise
rem close before you read the result. Called with an action -- from a terminal,
rem from CI, from a chained script -- it is just a hang. `save.bat` bare still
rem pauses, and so does the menu, because %~1 is empty on both.
if "%~1"=="" pause
exit /b %SAVE_ERROR%
