<#
.SYNOPSIS
  Authenticate Certum SimplySign Desktop on a Windows CI runner so that the
  cloud Code Signing certificate becomes usable by signtool (via the Certum
  CSP / virtual smart card) during the Tauri bundle step.

.DESCRIPTION
  The Open Source Code Signing certificate's private key lives in Certum's
  cloud HSM (SimplySign). To sign with it, SimplySign Desktop must be installed
  and logged in. Login requires a user/card ID plus a time-based OTP. We derive
  the OTP from the otpauth:// secret captured at enrolment, so no phone is
  needed and the whole thing runs unattended.

  After a successful login a virtual smart card is mounted and the certificate
  appears in Cert:\CurrentUser\My; signtool /sha1 <thumbprint> then signs via
  the cloud key (PIN cache keeps it usable for the rest of the run).

  Required environment variables (provided as GitHub Secrets):
    CERTUM_SIMPLYSIGN_USERID   - SimplySign user / card ID
    CERTUM_SIMPLYSIGN_PASSWORD - SimplySign account password
    CERTUM_OTP_SECRET          - the otpauth:// URI, or just its base32 secret
    CERTUM_CERT_SHA1           - expected certificate thumbprint (for verification)

  Sources / prior art (this GUI-login automation is the one part that cannot be
  tested outside a real Windows runner — expect to harden it on the first rc):
    https://www.devas.life/how-to-automate-signing-your-windows-app-with-certum/
    https://github.com/hpvb/certum-container
#>

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-Totp {
    # RFC 6238 TOTP (HMAC-SHA1, 6 digits, 30s step) from a base32 secret.
    param(
        [Parameter(Mandatory = $true)][string] $Base32Secret,
        [int] $Digits = 6,
        [int] $Period = 30
    )
    $alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
    $clean = $Base32Secret.Trim().TrimEnd('=').ToUpper() -replace '\s', ''
    $bits = ''
    foreach ($ch in $clean.ToCharArray()) {
        $idx = $alphabet.IndexOf($ch)
        if ($idx -lt 0) { continue }
        $bits += [Convert]::ToString($idx, 2).PadLeft(5, '0')
    }
    $bytes = New-Object System.Collections.Generic.List[byte]
    for ($i = 0; ($i + 8) -le $bits.Length; $i += 8) {
        $bytes.Add([Convert]::ToByte($bits.Substring($i, 8), 2))
    }
    $key = $bytes.ToArray()

    $counter = [int64][Math]::Floor([DateTimeOffset]::UtcNow.ToUnixTimeSeconds() / $Period)
    $counterBytes = [BitConverter]::GetBytes($counter)
    if ([BitConverter]::IsLittleEndian) { [Array]::Reverse($counterBytes) }

    $hmac = New-Object System.Security.Cryptography.HMACSHA1
    $hmac.Key = $key
    $hash = $hmac.ComputeHash($counterBytes)

    $offset = $hash[$hash.Length - 1] -band 0x0f
    $binary = (([int]$hash[$offset] -band 0x7f) -shl 24) -bor `
              (([int]$hash[$offset + 1] -band 0xff) -shl 16) -bor `
              (([int]$hash[$offset + 2] -band 0xff) -shl 8) -bor `
               ([int]$hash[$offset + 3] -band 0xff)
    $otp = $binary % [int][Math]::Pow(10, $Digits)
    return ([string]$otp).PadLeft($Digits, '0')
}

function Get-SecretFromOtpUri {
    # Accept either a full otpauth:// URI or a bare base32 secret.
    param([Parameter(Mandatory = $true)][string] $Value)
    if ($Value -match 'secret=([A-Za-z2-7=]+)') { return $Matches[1] }
    return $Value
}

# --- validate inputs --------------------------------------------------------
$userId   = $env:CERTUM_SIMPLYSIGN_USERID
$password = $env:CERTUM_SIMPLYSIGN_PASSWORD
$otpInput = $env:CERTUM_OTP_SECRET
$expected = $env:CERTUM_CERT_SHA1

foreach ($pair in @(
        @{ n = 'CERTUM_SIMPLYSIGN_USERID'; v = $userId },
        @{ n = 'CERTUM_SIMPLYSIGN_PASSWORD'; v = $password },
        @{ n = 'CERTUM_OTP_SECRET'; v = $otpInput },
        @{ n = 'CERTUM_CERT_SHA1'; v = $expected })) {
    if ([string]::IsNullOrWhiteSpace($pair.v)) {
        throw "Required secret '$($pair.n)' is empty. Set it in the repo's GitHub Actions secrets."
    }
}
$expected = ($expected -replace '[^0-9A-Fa-f]', '').ToUpper()
$secret = Get-SecretFromOtpUri -Value $otpInput

# --- install SimplySign Desktop --------------------------------------------
# NOTE: confirm/pin the exact installer URL + silent flags on the first rc run.
# Certum publishes SimplySign Desktop here: https://www.certum.eu/en/support_download_software/
$installerUrl = $env:SIMPLYSIGN_INSTALLER_URL
if ([string]::IsNullOrWhiteSpace($installerUrl)) {
    $installerUrl = 'https://files.certum.eu/software/SimplySign_Desktop/Windows/SimplySignDesktop-setup.exe'
}
$installer = Join-Path $env:RUNNER_TEMP 'SimplySignDesktop-setup.exe'
Write-Host "Downloading SimplySign Desktop from $installerUrl"
Invoke-WebRequest -Uri $installerUrl -OutFile $installer -UseBasicParsing
Write-Host 'Installing SimplySign Desktop silently...'
Start-Process -FilePath $installer -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait

# Locate the SimplySign Desktop executable.
$exeCandidates = @(
    "$env:ProgramFiles\Certum\SimplySign Desktop\SimplySignDesktop.exe",
    "${env:ProgramFiles(x86)}\Certum\SimplySign Desktop\SimplySignDesktop.exe"
)
$exe = $exeCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $exe) {
    $exe = Get-ChildItem -Path "$env:ProgramFiles", "${env:ProgramFiles(x86)}" -Filter 'SimplySignDesktop.exe' -Recurse -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $exe) { throw 'SimplySignDesktop.exe not found after install. Verify the installer URL/flags.' }
Write-Host "SimplySign Desktop: $exe"

# --- log in -----------------------------------------------------------------
# SimplySign Desktop is a GUI app; drive its login dialog via SendKeys. The
# field order (user id -> TAB -> password -> TAB -> OTP -> ENTER) is the working
# assumption and the most likely thing to need adjustment on the first rc.
Add-Type -AssemblyName System.Windows.Forms
Start-Process -FilePath $exe | Out-Null
Start-Sleep -Seconds 20

$otp = Get-Totp -Base32Secret $secret
$wshell = New-Object -ComObject WScript.Shell
$null = $wshell.AppActivate('SimplySign')
Start-Sleep -Seconds 2
[System.Windows.Forms.SendKeys]::SendWait($userId)
[System.Windows.Forms.SendKeys]::SendWait('{TAB}')
[System.Windows.Forms.SendKeys]::SendWait($password)
[System.Windows.Forms.SendKeys]::SendWait('{TAB}')
[System.Windows.Forms.SendKeys]::SendWait($otp)
[System.Windows.Forms.SendKeys]::SendWait('{ENTER}')

# --- wait for the certificate to appear in the store ------------------------
Write-Host "Waiting for certificate $expected to appear in Cert:\CurrentUser\My ..."
$deadline = (Get-Date).AddMinutes(3)
$found = $false
while ((Get-Date) -lt $deadline) {
    $match = Get-ChildItem -Path Cert:\CurrentUser\My -ErrorAction SilentlyContinue |
        Where-Object { $_.Thumbprint -eq $expected }
    if ($match) { $found = $true; break }
    Start-Sleep -Seconds 5
}

if (-not $found) {
    Write-Host '--- DIAGNOSTICS: certificates currently in CurrentUser\My ---'
    Get-ChildItem -Path Cert:\CurrentUser\My -ErrorAction SilentlyContinue |
        Format-Table Thumbprint, Subject -AutoSize | Out-String | Write-Host
    throw "Code signing certificate $expected did not appear after SimplySign login. " +
          'Adjust the login automation (field order/timing) or the installer URL, then re-run.'
}

Write-Host "OK: certificate $expected is available for signing."
