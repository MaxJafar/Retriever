param([Parameter(Mandatory=$true)][string]$Payload)
$ErrorActionPreference='Stop'
$RetrieverRoot=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$RetrieverVswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$RetrieverVisualStudio=(& $RetrieverVswhere -latest -products '*' -property installationPath).Trim()
$RetrieverRedist=Get-ChildItem -LiteralPath (Join-Path $RetrieverVisualStudio 'VC/Redist/MSVC') -Directory | Where-Object {$_.Name -match '^\d+\.\d+\.\d+$'} | Sort-Object {[version]$_.Name} -Descending | Select-Object -First 1
$RetrieverCrt=Join-Path $RetrieverRedist.FullName 'x64/Microsoft.VC143.CRT'
if(!(Test-Path -LiteralPath (Join-Path $RetrieverCrt 'vcruntime140.dll'))){throw 'Missing C++ redistributable'}
New-Item -ItemType Directory -Force (Join-Path $Payload 'notices'),(Join-Path $Payload 'engine') | Out-Null
foreach($RetrieverFile in Get-ChildItem -LiteralPath $RetrieverCrt -File -Filter '*.dll'){
 Copy-Item -LiteralPath $RetrieverFile.FullName -Destination $Payload
 Copy-Item -LiteralPath $RetrieverFile.FullName -Destination (Join-Path $Payload 'engine')
}
$RetrieverNotice=Get-ChildItem -LiteralPath (Join-Path $RetrieverVisualStudio 'Licenses') -Recurse -File -Filter Redist.txt | Select-Object -First 1
Copy-Item -LiteralPath $RetrieverNotice.FullName -Destination (Join-Path $Payload 'notices/MICROSOFT-CPP-REDIST.txt')
$RetrieverSdk=Join-Path $RetrieverRoot 'artifacts/retriever-rust/build-tools/dotnet'
if(!(Test-Path -LiteralPath (Join-Path $RetrieverSdk 'ThirdPartyNotices.txt'))){$RetrieverSdk=Split-Path (Get-Command dotnet -ErrorAction Stop).Source}
Copy-Item -LiteralPath (Join-Path $RetrieverSdk 'ThirdPartyNotices.txt') -Destination (Join-Path $Payload 'notices/DOTNET-THIRD-PARTY.txt')
Copy-Item -LiteralPath (Join-Path $RetrieverSdk 'LICENSE.txt') -Destination (Join-Path $Payload 'notices/DOTNET-LICENSE.txt')
$RetrieverNuget=Join-Path $RetrieverRoot 'artifacts/retriever-rust/build-tools/cli/.nuget/packages'
if($env:NUGET_PACKAGES){$RetrieverNuget=$env:NUGET_PACKAGES}
foreach($RetrieverName in @('microsoft.windowsappsdk','microsoft.windows.sdk.net.ref')){
 $RetrieverBase=Join-Path $RetrieverNuget $RetrieverName
 if(!(Test-Path -LiteralPath $RetrieverBase)){throw "Missing package notices: $RetrieverName"}
 foreach($RetrieverNotice in Get-ChildItem -LiteralPath $RetrieverBase -Recurse -File | Where-Object {$_.Name -match '^(LICENSE|COPYING|NOTICE)'}){
  $RetrieverDest=Join-Path $Payload ('notices/nuget/'+$RetrieverName+'/'+$RetrieverNotice.Directory.Name)
  New-Item -ItemType Directory -Force $RetrieverDest | Out-Null
  Copy-Item -LiteralPath $RetrieverNotice.FullName -Destination $RetrieverDest
 }
}
