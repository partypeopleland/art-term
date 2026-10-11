param(
    [Parameter(Mandatory=$true)][int]$AppProcessId,
    [Parameter(Mandatory=$true)][ValidateSet('choose','cancel')][string]$Action,
    [Parameter(Mandatory=$true)][string]$RunRoot,
    [string]$TestFile
)
$ErrorActionPreference = 'Stop'
$allowedRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../target/windows-regression')) + [IO.Path]::DirectorySeparatorChar
$resolvedRun = [IO.Path]::GetFullPath($RunRoot)
if (-not $resolvedRun.StartsWith($allowedRoot, [StringComparison]::OrdinalIgnoreCase) -or (Split-Path $resolvedRun -Leaf) -notmatch '^run-[a-f0-9]{32}$') {
    throw 'Run root must be a unique regression run directory.'
}
foreach ($candidate in @($resolvedRun, $TestFile)) {
    if (-not $candidate) { continue }
    $cursor = [IO.Path]::GetFullPath($candidate)
    while ($cursor -and $cursor.Length -ge $allowedRoot.TrimEnd('\').Length) {
        if (Test-Path -LiteralPath $cursor) {
            if ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Reparse point is not allowed in picker paths.' }
        }
        $cursor = Split-Path -Path $cursor -Parent
    }
}
# Operates only a dialog owned by the explicitly supplied test GUI process.
# Do not use global keystrokes, enumerate other apps, or output UI field values.
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class TransferPickerWindows {
  public delegate bool Callback(IntPtr window, IntPtr data);
  [DllImport("user32.dll")] static extern bool EnumWindows(Callback callback, IntPtr data);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr window, StringBuilder text, int maximum);
  [DllImport("user32.dll")] static extern bool IsChild(IntPtr parent, IntPtr child);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr window);
  [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wp, string value, uint flags, uint timeout, out IntPtr result);
  [DllImport("user32.dll", SetLastError=true)] static extern bool PostMessage(IntPtr window, uint message, IntPtr wp, IntPtr lp);
  static void Check(IntPtr parent, IntPtr child, uint pid, string expectedClass, int id) {
    uint process; GetWindowThreadProcessId(child, out process);
    var name = new StringBuilder(256); GetClassName(child, name, name.Capacity);
    if (!IsChild(parent,child) || process != pid || name.ToString() != expectedClass || GetDlgCtrlID(child) != id)
      throw new InvalidOperationException("Owned picker control identity mismatch");
  }
  public static void Filename(IntPtr parent, IntPtr edit, uint pid, string value) {
    int id=GetDlgCtrlID(edit);
    if(id!=1001 && id!=1148) throw new InvalidOperationException("Unexpected filename control ID");
    Check(parent,edit,pid,"Edit",id); IntPtr result;
    if(SendMessageTimeout(edit,0x000C,IntPtr.Zero,value,2,3000,out result)==IntPtr.Zero)
      throw new InvalidOperationException("Owned filename control did not accept text");
  }
  public static void Click(IntPtr parent, IntPtr button, uint pid, int id) {
    Check(parent,button,pid,"Button",id);
    if(!PostMessage(button,0x00F5,IntPtr.Zero,IntPtr.Zero)) throw new InvalidOperationException("Owned picker click failed");
  }
  public static IntPtr[] Dialogs(uint pid) {
    var found = new List<IntPtr>();
    EnumWindows((window, data) => { uint process; GetWindowThreadProcessId(window, out process);
      if (process == pid) { var name = new StringBuilder(256); GetClassName(window, name, name.Capacity);
        if (name.ToString() == "#32770") found.Add(window); }
      return true; }, IntPtr.Zero);
    return found.ToArray();
  }
}
'@
$process = Get-Process -Id $AppProcessId -ErrorAction Stop
if ($process.ProcessName -ne 'art-term-gui') { throw 'Expected owned art-term GUI process.' }
if ($Action -eq 'choose') {
    $testRoot = [IO.Path]::GetFullPath($RunRoot) + [IO.Path]::DirectorySeparatorChar
    $resolvedFile = [IO.Path]::GetFullPath($TestFile)
    if (-not $resolvedFile.StartsWith($testRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Picker test file must remain inside task-owned target directory.'
    }
}
$pidCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, $AppProcessId)
$classCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, '#32770')
$dialogCondition = New-Object System.Windows.Automation.AndCondition($pidCondition, $classCondition)
$deadline = [DateTime]::UtcNow.AddSeconds(15)
$dialog = $null
do {
    # Enumerate actual top-level HWNDs; UIA's tree also includes #32770
    # explorer subpanes that are not the complete file dialog.
    $handles = @([TransferPickerWindows]::Dialogs([uint32]$AppProcessId))
    if ($handles.Count -eq 1) {
        $dialog = [System.Windows.Automation.AutomationElement]::FromHandle($handles[0])
    }
    if ($null -eq $dialog) { Start-Sleep -Milliseconds 150 }
} while ($null -eq $dialog -and [DateTime]::UtcNow -lt $deadline)
if ($null -eq $dialog) {
    $owned = [System.Windows.Automation.AutomationElement]::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $pidCondition)
    $classes = @($owned | ForEach-Object { $_.Current.ClassName })
    @{ failure = 'Owned native picker did not appear.'; ownedWindowClasses = $classes } | ConvertTo-Json -Compress
    exit 1
}

if ($Action -eq 'choose') {
    $editCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)
    $filename = $null
    do {
        $edits = $dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants, $editCondition)
        foreach ($edit in $edits) {
            $id = $edit.Current.AutomationId
            $parent = [System.Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($edit)
            $parentId = if ($parent) { $parent.Current.AutomationId } else { '' }
            if ($id -in @('1001','1148') -or $parentId -in @('1001','1148','FileNameControlHost')) { $filename = $edit; break }
        }
        if ($null -eq $filename) {
            $all = $dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
            foreach ($control in $all) {
                if ($control.Current.AutomationId -in @('1001','1148') -and $control.Current.ClassName -eq 'Edit') { $filename = $control; break }
                if ($control.Current.AutomationId -in @('1001','1148','FileNameControlHost')) {
                    $pattern = $null
                    if ($control.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) { $filename = $control; break }
                }
            }
        }
        if ($null -eq $filename) { Start-Sleep -Milliseconds 150 }
    } while ($null -eq $filename -and [DateTime]::UtcNow -lt $deadline)
    if ($null -eq $filename) {
        $ids = @($all | ForEach-Object { [pscustomobject]@{id=$_.Current.AutomationId; type=$_.Current.ControlType.ProgrammaticName; class=$_.Current.ClassName} } | Sort-Object id,type,class -Unique)
        @{ failure = 'Native filename control was not found'; controlMetadata = $ids } | ConvertTo-Json -Depth 4 -Compress
        exit 1
    }
    $value = $null
    if ($filename.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$value)) {
        $value.SetValue($resolvedFile)
    } else {
        # This Windows provider exposes classic controls as Pane without patterns.
        # Send only to the verified owned dialog's exact native Edit handle.
        [TransferPickerWindows]::Filename([IntPtr]$dialog.Current.NativeWindowHandle, [IntPtr]$filename.Current.NativeWindowHandle, [uint32]$AppProcessId, $resolvedFile)
    }
}
$buttonId = if ($Action -eq 'cancel') { '2' } else { '1' }
$buttonIdCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::AutomationIdProperty, $buttonId)
$buttonClassCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ClassNameProperty, 'Button')
$buttonCondition = New-Object System.Windows.Automation.AndCondition($buttonIdCondition, $buttonClassCondition)
$button = $dialog.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
if ($null -eq $button) { throw 'Native picker action button not found.' }
$invoke = $null
if ($button.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$invoke)) {
    $invoke.Invoke()
} else {
    [TransferPickerWindows]::Click([IntPtr]$dialog.Current.NativeWindowHandle, [IntPtr]$button.Current.NativeWindowHandle, [uint32]$AppProcessId, [int]$buttonId)
}
Write-Output ('{"native_picker_action":"' + $Action + '","invoked":true}')
