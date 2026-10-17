using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Retriever.Native;

/// <summary>Keeps an explicitly configured, loopback-only SSH forwarding alive.</summary>
sealed class SharedLibraryRelay:IDisposable {
    readonly CancellationTokenSource stop=new();
    readonly string state,host,sshConfig;
    readonly int localPort,remotePort;
    readonly Mutex singleton;
    readonly bool ownsMutex;
    readonly SafeFileHandle? job;
    Process? child;
    public SharedLibraryRelay(JsonObject settings,string state,int localPort){
        this.state=state;this.localPort=localPort;
        host=settings["host"]?.ToString()??"";
        sshConfig=settings["ssh_config"]?.ToString()??"";
        remotePort=settings["remote_port"]?.GetValue<int>()??0;
        if(!System.Text.RegularExpressions.Regex.IsMatch(host,@"^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$")||!Path.IsPathFullyQualified(sshConfig)||!File.Exists(sshConfig)||remotePort<1024||remotePort>65535)
            throw new InvalidOperationException("The shared library SSH connection is invalid.");
        var identity=Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(state+"|"+host+"|"+remotePort)))[..24];
        singleton=new Mutex(true,"Local\\RetrieverSharedLibrary"+identity,out ownsMutex);
        if(ownsMutex){
            job=CreateJobObjectW(0,null);
            var limits=new ExtendedLimits{Basic=new BasicLimits{Flags=0x2000}};
            if(job.IsInvalid||!SetInformationJobObject(job,9,ref limits,(uint)Marshal.SizeOf<ExtendedLimits>())){
                var error=Marshal.GetLastWin32Error();job.Dispose();singleton.ReleaseMutex();singleton.Dispose();
                throw new System.ComponentModel.Win32Exception(error,"Could not bind the shared connection to Retriever's lifetime.");
            }
            _=Task.Run(Run);
        }
    }
    void Status(string phase,string detail=""){
        try {
            var path=Path.Combine(state,"shared-library-relay.json");var temporary=path+".new";
            var value=new JsonObject{["state"]=phase,["host"]=host,["remote_port"]=remotePort,["local_port"]=localPort,["pid"]=child?.Id,["updated"]=DateTimeOffset.UtcNow.ToString("O"),["detail"]=detail};
            File.WriteAllText(temporary,value.ToJsonString());File.Move(temporary,path,true);
        } catch { /* Connection supervision must not interrupt file browsing. */ }
    }
    async Task Run(){
        var delay=3;
        try {
            while(!stop.IsCancellationRequested){
                try {
                    var info=new ProcessStartInfo(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows),"System32","OpenSSH","ssh.exe")){
                        UseShellExecute=false,CreateNoWindow=true,RedirectStandardError=true,RedirectStandardOutput=true
                    };
                    foreach(var argument in new[]{"-F",sshConfig,"-N","-o","BatchMode=yes","-o","StrictHostKeyChecking=yes","-o","ExitOnForwardFailure=yes","-o","ConnectTimeout=10","-o","ServerAliveInterval=30","-o","ServerAliveCountMax=3","-R",$"127.0.0.1:{remotePort}:127.0.0.1:{localPort}",host})info.ArgumentList.Add(argument);
                    child=Process.Start(info)??throw new InvalidOperationException("SSH did not start.");
                    if(job==null||!AssignProcessToJobObject(job,child.Handle))throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
                    Status("connecting");
                    var errors=child.StandardError.ReadToEndAsync(stop.Token);
                    var output=child.StandardOutput.ReadToEndAsync(stop.Token);
                    await Task.Delay(1500,stop.Token);
                    if(!child.HasExited){Status("forwarding");delay=3;}
                    await child.WaitForExitAsync(stop.Token);
                    var detail=await errors;await output;
                    Status("reconnecting",detail.Length>1000?detail[..1000]:detail);
                } catch(OperationCanceledException) when(stop.IsCancellationRequested){break;}
                catch(Exception error){Status("reconnecting",error.Message);}
                finally {
                    if(child is {HasExited:false})try{child.Kill();}catch{}
                    child?.Dispose();child=null;
                }
                await Task.Delay(TimeSpan.FromSeconds(delay),stop.Token);delay=Math.Min(30,delay*2);
            }
        } catch(OperationCanceledException) when(stop.IsCancellationRequested){}
        finally {Status("stopped");}
    }
    public void Dispose(){stop.Cancel();job?.Dispose();if(ownsMutex)singleton.ReleaseMutex();singleton.Dispose();}
    [StructLayout(LayoutKind.Sequential)] struct BasicLimits {
        public long ProcessTime,JobTime;public uint Flags;public nuint MinimumWorkingSet,MaximumWorkingSet;
        public uint ActiveProcesses;public nuint Affinity;public uint Priority,Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] struct IoCounters {public ulong ReadOps,WriteOps,OtherOps,ReadBytes,WriteBytes,OtherBytes;}
    [StructLayout(LayoutKind.Sequential)] struct ExtendedLimits {
        public BasicLimits Basic;public IoCounters Io;public nuint ProcessMemory,JobMemory,PeakProcessMemory,PeakJobMemory;
    }
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern SafeFileHandle CreateJobObjectW(nint attributes,string? name);
    [DllImport("kernel32.dll",SetLastError=true)] [return:MarshalAs(UnmanagedType.Bool)] static extern bool SetInformationJobObject(SafeFileHandle job,int kind,ref ExtendedLimits value,uint length);
    [DllImport("kernel32.dll",SetLastError=true)] [return:MarshalAs(UnmanagedType.Bool)] static extern bool AssignProcessToJobObject(SafeFileHandle job,nint process);
}
