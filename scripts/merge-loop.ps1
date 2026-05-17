#!/usr/bin/env pwsh
# merge-loop.ps1
# Continuously reviews branches from Codex/Claude and merges acceptable changes into master.
# Usage: .\scripts\merge-loop.ps1 [-Once] [-DryRun] [-IntervalSeconds 300]
#
# What it does every cycle:
#   1. Fetch all remote branches
#   2. Detect codex/* and claude/* branches not yet merged into master
#   3. For each branch: diff stats, cargo test, if green -> merge into master
#   4. Update HANDOFF.md with session log entry
#   5. Wait for next interval

param(
    [switch]$Once,
    [switch]$DryRun,
    [int]$IntervalSeconds = 300
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

# -----------------------------------------------------------------------
# Helpers
# -----------------------------------------------------------------------
function Write-Log {
    param([string]$Level, [string]$Message)
    $ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $color = switch ($Level) {
        "INFO"  { "Cyan"   }
        "OK"    { "Green"  }
        "WARN"  { "Yellow" }
        "FAIL"  { "Red"    }
        "SKIP"  { "DarkGray" }
        default { "White"  }
    }
    Write-Host "[$ts] [$Level] $Message" -ForegroundColor $color
}

function Invoke-Git {
    param([string[]]$Args)
    $result = & git @Args 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Args -join ' ') failed: $result"
    }
    return $result
}

function Get-UnmergedAIBranches {
    # Returns local+remote branches matching codex/* or claude/* that are not merged into master
    $merged = & git branch --merged master -a 2>&1 | ForEach-Object { $_.Trim().TrimStart("* ") }
    $all    = & git branch -a 2>&1 | ForEach-Object { $_.Trim().TrimStart("* ") }

    $candidates = $all | Where-Object {
        ($_ -match "^(remotes/origin/)?(codex|claude)/") -and
        ($merged -notcontains $_)
    }
    return $candidates
}

function Get-BranchDiffSummary {
    param([string]$Branch)
    $stat = & git diff "master...$Branch" --stat 2>&1
    return ($stat | Select-Object -Last 5) -join "`n"
}

function Run-CargoTest {
    param([string]$Branch)
    # Stash, checkout branch, test, restore
    $stashNeeded = (& git status --short 2>&1) -ne ""
    if ($stashNeeded) { & git stash push -q -m "merge-loop-stash" | Out-Null }

    try {
        & git checkout $Branch -q 2>&1 | Out-Null
        $output = & cargo test 2>&1
        $exitCode = $LASTEXITCODE
        $summary = ($output | Select-String "test result") -join " "
        return [PSCustomObject]@{
            Passed  = ($exitCode -eq 0)
            Output  = $output -join "`n"
            Summary = $summary
        }
    } finally {
        & git checkout master -q 2>&1 | Out-Null
        if ($stashNeeded) { & git stash pop -q 2>&1 | Out-Null }
    }
}

function Merge-Branch {
    param([string]$Branch, [string]$CommitMessage)
    if ($DryRun) {
        Write-Log "SKIP" "[DRY-RUN] Would merge $Branch"
        return $true
    }
    try {
        Invoke-Git @("merge", "--no-ff", $Branch, "-m", $CommitMessage) | Out-Null
        return $true
    } catch {
        Write-Log "FAIL" "Merge of $Branch failed: $_"
        & git merge --abort 2>&1 | Out-Null
        return $false
    }
}

function Append-HandoffLog {
    param([string]$Entry)
    if ($DryRun) { return }
    $handoff = Join-Path $repoRoot "HANDOFF.md"
    if (-not (Test-Path $handoff)) { return }
    $content = Get-Content $handoff -Raw
    if ($content -match "## Session Log") {
        $updated = $content -replace "(## Session Log)", "`$1`n`n$Entry"
        Set-Content $handoff $updated -NoNewline
    } else {
        Add-Content $handoff "`n## Session Log`n`n$Entry"
    }
}

# -----------------------------------------------------------------------
# Evaluate a single branch and merge if good
# Returns: "merged" | "failed_tests" | "nothing_new" | "merge_conflict"
# -----------------------------------------------------------------------
function Process-Branch {
    param([string]$Branch)

    Write-Log "INFO" "Evaluating branch: $Branch"

    # Normalise: strip remotes/origin/ prefix for checkout
    $localBranch = $Branch -replace "^remotes/origin/", ""

    # Ensure we have local tracking
    $exists = & git branch 2>&1 | ForEach-Object { $_.Trim() } | Where-Object { $_ -eq $localBranch }
    if (-not $exists) {
        & git checkout -b $localBranch "origin/$($localBranch -replace '^[^/]+/', '')" -q 2>&1 | Out-Null
    }

    # Check if there are real code changes
    $diffFiles = & git diff "master...$localBranch" --name-only 2>&1
    $codeFiles = $diffFiles | Where-Object { $_ -match "\.(rs|toml|html|js|css)$" }
    if (-not $codeFiles) {
        Write-Log "SKIP" "No code changes in $localBranch, skipping"
        return "nothing_new"
    }

    $diffSummary = Get-BranchDiffSummary $localBranch
    Write-Log "INFO" "Diff summary:`n$diffSummary"

    # Run tests on the branch
    Write-Log "INFO" "Running cargo test on $localBranch ..."
    $testResult = Run-CargoTest $localBranch
    if (-not $testResult.Passed) {
        Write-Log "FAIL" "Tests FAILED on $localBranch"
        Write-Log "FAIL" $testResult.Summary
        $date = Get-Date -Format "yyyy-MM-dd HH:mm"
        Append-HandoffLog "- `$date` merge-loop: **$localBranch** REJECTED (tests failed)"
        return "failed_tests"
    }

    Write-Log "OK" "Tests PASSED on $localBranch: $($testResult.Summary)"

    # Merge!
    $commitMsg = "Merge branch '$localBranch' via merge-loop (auto)"
    $merged = Merge-Branch $localBranch $commitMsg

    if ($merged) {
        $date = Get-Date -Format "yyyy-MM-dd HH:mm"
        Append-HandoffLog "- `$date` merge-loop: **$localBranch** merged into master"
        Write-Log "OK" "Merged $localBranch into master"
        return "merged"
    } else {
        return "merge_conflict"
    }
}

# -----------------------------------------------------------------------
# Main loop
# -----------------------------------------------------------------------
Write-Log "INFO" "merge-loop started (repo: $repoRoot, interval: ${IntervalSeconds}s, dryRun: $DryRun)"

$cycleCount = 0
while ($true) {
    $cycleCount++
    Write-Log "INFO" "=== Cycle #$cycleCount ==="

    # Sync remote
    try {
        Write-Log "INFO" "Fetching remotes..."
        Invoke-Git @("fetch", "--all", "--prune") | Out-Null
    } catch {
        Write-Log "WARN" "Fetch failed: $_"
    }

    # Ensure we're on master
    $currentBranch = (& git rev-parse --abbrev-ref HEAD 2>&1).Trim()
    if ($currentBranch -ne "master") {
        Write-Log "WARN" "Not on master (was on $currentBranch), checking out master"
        & git checkout master -q 2>&1 | Out-Null
    }

    # Pull latest master
    try {
        Invoke-Git @("pull", "--ff-only", "origin", "master") | Out-Null
        Write-Log "INFO" "master is up to date"
    } catch {
        Write-Log "WARN" "Could not pull master: $_"
    }

    # Find branches to process
    $branches = Get-UnmergedAIBranches
    if (-not $branches) {
        Write-Log "SKIP" "No unmerged AI branches found"
    } else {
        Write-Log "INFO" "Found $($branches.Count) unmerged AI branch(es): $($branches -join ', ')"
        foreach ($branch in $branches) {
            $outcome = Process-Branch $branch
            Write-Log "INFO" "  -> $branch : $outcome"
        }
    }

    # Push master if we merged something (skip if dry run)
    if (-not $DryRun) {
        $localHead  = & git rev-parse HEAD 2>&1
        $remoteHead = & git rev-parse "origin/master" 2>&1
        if ($localHead -ne $remoteHead) {
            try {
                Write-Log "INFO" "Pushing master to origin..."
                Invoke-Git @("push", "origin", "master") | Out-Null
                Write-Log "OK" "Push succeeded"
            } catch {
                Write-Log "WARN" "Push failed (will retry next cycle): $_"
            }
        }
    }

    if ($Once) {
        Write-Log "INFO" "Done (--Once specified). Exiting."
        break
    }

    Write-Log "INFO" "Sleeping ${IntervalSeconds}s until next cycle..."
    Start-Sleep -Seconds $IntervalSeconds
}
