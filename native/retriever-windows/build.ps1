param([switch]$Testing,[string]$OutputDirectory='',[string]$EnginePath='')
$ErrorActionPreference='Stop'
$RetrieverRoot=(Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$RetrieverSdk=Join-Path $RetrieverRoot 'artifacts/retriever-rust/build-tools/dotnet/dotnet.exe'
if(!(Test-Path -LiteralPath $RetrieverSdk)){$RetrieverSdk=(Get-Command dotnet -ErrorAction Stop).Source}
$env:DOTNET_ROOT=Split-Path $RetrieverSdk
$env:DOTNET_CLI_TELEMETRY_OPTOUT='1'
$env:DOTNET_CLI_HOME=Join-Path $RetrieverRoot 'artifacts/retriever-rust/build-tools/cli'
$RetrieverPublish=if($OutputDirectory){[IO.Path]::GetFullPath($OutputDirectory)}else{Join-Path $PSScriptRoot $(if($Testing){'publish-tests'}else{'publish'})}
$RetrieverBuildArgs=@('publish',(Join-Path $PSScriptRoot 'Retriever.csproj'),'-c','Release','-p:Platform=x64','-o',$RetrieverPublish)
if($Testing){$RetrieverBuildArgs+='-p:DefineConstants=RETRIEVER_TESTING'}
& $RetrieverSdk @RetrieverBuildArgs
if($LASTEXITCODE -ne 0){throw 'Native candidate build failed'}
$RetrieverEngine=if($EnginePath){[IO.Path]::GetFullPath($EnginePath)}else{Join-Path $RetrieverRoot 'retriever-rs/target/release/retriever.exe'}
if(Test-Path -LiteralPath $RetrieverEngine){
  New-Item -ItemType Directory -Force (Join-Path $RetrieverPublish 'engine') | Out-Null
  $RetrieverBundledEngine=Join-Path $RetrieverPublish 'engine/retriever.exe'
  if(!(Test-Path -LiteralPath $RetrieverBundledEngine) -or (Get-FileHash -LiteralPath $RetrieverBundledEngine).Hash -ne (Get-FileHash -LiteralPath $RetrieverEngine).Hash){
    Copy-Item -LiteralPath $RetrieverEngine -Destination $RetrieverBundledEngine
  }
}
$RetrieverUpdater=Join-Path (Split-Path $RetrieverEngine) 'retriever-updater.exe'
if(Test-Path -LiteralPath $RetrieverUpdater){Copy-Item -LiteralPath $RetrieverUpdater -Destination (Join-Path $RetrieverPublish 'engine/retriever-updater.exe') -Force}
