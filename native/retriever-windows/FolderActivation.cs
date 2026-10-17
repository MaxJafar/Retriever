using System.IO.Pipes;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace Retriever.Native;
// Per build/profile, same-user IPC. A second launch forwards to the existing window.
sealed class FolderActivation:IDisposable {
    readonly Mutex mutex;readonly string pipe;readonly bool owner;readonly CancellationTokenSource stop=new();
    public FolderActivation(){var identity=Environment.UserDomainName+"/"+Environment.UserName+"|"+AppContext.BaseDirectory+"|"+Environment.GetEnvironmentVariable("RETRIEVER_PROFILE");pipe="Retriever-"+Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(identity)))[..24];mutex=new Mutex(false,"Local\\"+pipe);try{owner=mutex.WaitOne(0);}catch(AbandonedMutexException){owner=true;}}
    public bool IsOwner=>owner;
    public static string? Folder(string[] args){var index=Array.IndexOf(args,"--open-folder");if(index<0)return null;if(index+1>=args.Length)throw new ArgumentException("Choose a folder to open.");var path=args[index+1];if(path.Length>4096||path.StartsWith(@"\\")||!Path.IsPathFullyQualified(path)||!Directory.Exists(path))throw new ArgumentException("Choose an existing local folder.");return Path.GetFullPath(path);}
    public async Task Forward(string[] args){using var client=new NamedPipeClientStream(".",pipe,PipeDirection.Out,PipeOptions.Asynchronous|PipeOptions.CurrentUserOnly);await client.ConnectAsync(5000);await using var writer=new StreamWriter(client);await writer.WriteLineAsync(JsonSerializer.Serialize(args));}
    public async Task Listen(Action<string[]> activate){while(!stop.IsCancellationRequested){try{using var server=new NamedPipeServerStream(pipe,PipeDirection.In,1,PipeTransmissionMode.Byte,PipeOptions.Asynchronous|PipeOptions.CurrentUserOnly);await server.WaitForConnectionAsync(stop.Token);using var limit=CancellationTokenSource.CreateLinkedTokenSource(stop.Token);limit.CancelAfter(2000);using var reader=new StreamReader(server);var chars=new char[8192];var count=0;while(count<chars.Length){var read=await reader.ReadAsync(chars.AsMemory(count,1),limit.Token);if(read==0||chars[count]=='\n')break;count+=read;}if(count==chars.Length)continue;var args=JsonSerializer.Deserialize<string[]>(new string(chars,0,count));if(args is {Length:<=8})activate(args);}catch(OperationCanceledException){}catch(IOException){}catch(JsonException){}}}
    public void Dispose(){stop.Cancel();if(owner)mutex.ReleaseMutex();mutex.Dispose();stop.Dispose();}
}
public sealed partial class MainWindow {
    public async void ActivateRequest(string[] args){if(!await workspaceReady.Task)return;await Guard(async()=>{RestoreMain();var path=FolderActivation.Folder(args);if(path!=null){if(api.Mode=="client")throw new InvalidOperationException("Local folders require this computer's coordinator.");await Browse(status?["local_device"]?.ToString()??"local",path);}else if(args.Contains("--quick"))ToggleQuick();});}
}
