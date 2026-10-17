using System.Diagnostics;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Retriever.Native;

// Update requests use a separate credential-free helper, never the library's
// authenticated HTTP client. No file names, search queries or tokens are sent.
internal sealed class UpdateClient {
    readonly string directory,repository,helper;
    JsonObject preferences;
    Process? process;
    public event Action? Changed;
    public bool Busy{get;private set;}
    public bool Downloading{get;private set;}
    public string Message{get;private set;}="Check for new Retriever releases.";
    public JsonNode? Available{get;private set;}
    public string Current=>System.Reflection.Assembly.GetExecutingAssembly().GetCustomAttributes(typeof(System.Reflection.AssemblyInformationalVersionAttribute),false).Cast<System.Reflection.AssemblyInformationalVersionAttribute>().Single().InformationalVersion.Split('+')[0];
    public bool Automatic{get=>preferences["automatic"]?.GetValue<bool>()??true;set{preferences["automatic"]=value;Save();}}
    public bool Preview{get=>preferences["preview"]?.GetValue<bool>()??Current.Contains("alpha");set{preferences["preview"]=value;Available=null;Save();Changed?.Invoke();}}
    public bool Notify=>Available!=null&&Available["version"]?.ToString()!=preferences["skipped"]?.ToString()&&(preferences["later_until"]?.GetValue<long>()??0)<=DateTimeOffset.UtcNow.ToUnixTimeSeconds();
    public string? Receipt=>Available?["version"]?.ToString()==preferences["download_version"]?.ToString()?preferences["receipt"]?.ToString():null;
    public string Releases=>"https://github.com/"+repository+"/releases";
    public UpdateClient(string state){
        directory=Path.Combine(state,"updates");helper=Path.Combine(AppContext.BaseDirectory,"engine","retriever-updater.exe");
        var config=JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory,"repository.json")))!;
        repository=config["repository"]?.ToString()??throw new InvalidOperationException("The release repository is not configured.");
        if(!System.Text.RegularExpressions.Regex.IsMatch(repository,@"^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$"))throw new InvalidOperationException("Invalid release repository.");
        try{preferences=JsonNode.Parse(File.ReadAllText(Path.Combine(directory,"preferences.json")))!.AsObject();}catch{preferences=new();}
    }
    void Save(){try{Directory.CreateDirectory(directory);var path=Path.Combine(directory,"preferences.json");var temporary=path+"."+Guid.NewGuid().ToString("N");File.WriteAllText(temporary,preferences.ToJsonString());File.Move(temporary,path,true);}catch(Exception error){Message="Update preferences could not be saved: "+error.Message;Changed?.Invoke();}}
    public void Later(){preferences["later_until"]=DateTimeOffset.UtcNow.AddDays(1).ToUnixTimeSeconds();Save();Changed?.Invoke();}
    public void Skip(){preferences["skipped"]=Available?["version"]?.DeepClone();Save();Changed?.Invoke();}
    public void Cancel(){if(Downloading&&process is {HasExited:false})try{process.StandardInput.WriteLine("CANCEL");process.StandardInput.Flush();}catch(InvalidOperationException){}catch(IOException){}}
    async Task<JsonNode> Run(params string[] arguments){
        if(!File.Exists(helper))throw new InvalidOperationException("This build does not include the update helper.");
        var start=new ProcessStartInfo(helper){UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,RedirectStandardInput=true};
        start.ArgumentList.Add("--repository");start.ArgumentList.Add(repository);foreach(var argument in arguments)start.ArgumentList.Add(argument);
        using var child=Process.Start(start)??throw new InvalidOperationException("The update helper could not start.");process=child;
        var errors=child.StandardError.ReadToEndAsync();JsonNode? result=null;
        try{
            while(await child.StandardOutput.ReadLineAsync() is string line){var value=JsonNode.Parse(line);if(value?["event"]?.ToString()=="progress"){Message=$"Downloading update - {100*(value["received"]?.GetValue<double>()??0)/(value["total"]?.GetValue<double>()??1):F0}%";Changed?.Invoke();}else result=value;}
            await child.WaitForExitAsync();var error=await errors;
            if(child.ExitCode!=0){string detail;try{detail=JsonNode.Parse(error)?["error"]?.ToString()??"Update check failed.";}catch{detail="The update helper could not finish.";}throw new InvalidOperationException(detail);}
            return result??throw new InvalidOperationException("The update helper returned no result.");
        }finally{process=null;}
    }
    public async Task Check(bool manual){
        if(Busy||(!manual&&!Automatic))return;Busy=true;Message="Checking GitHub for updates...";Changed?.Invoke();
        try{var arguments=new List<string>{"check","--current",Current,"--platform","windows-x64","--cache",Path.Combine(directory,"releases.json")};if(Preview)arguments.Add("--preview");if(manual)arguments.Add("--force");var reply=await Run(arguments.ToArray());Available=reply["update"]?.DeepClone();Message=Available!=null?"Retriever "+Available["version"]+" is available.":reply["message"]?.ToString()??"You're up to date.";if(manual){preferences["later_until"]=0;preferences["skipped"]=null;Save();}}
        catch(Exception error){Message=error.Message;}finally{Busy=false;Changed?.Invoke();}
    }
    public async Task Download(){
        if(Busy||Available==null)return;Busy=true;Downloading=true;Changed?.Invoke();
        try{var target=Path.Combine(directory,"downloads",Guid.NewGuid().ToString("N"));var args=new List<string>{"download","--current",Current,"--platform","windows-x64","--release-id",Available["release_id"]!.ToString(),"--output",target};if(Preview)args.Add("--preview");var result=(await Run(args.ToArray()))["download"]!;preferences["download_version"]=result["version"]?.DeepClone();preferences["receipt"]=Path.Combine(target,"download.json");Save();Message="Update downloaded and verified. Install when you're ready.";}
        catch(Exception error){Message=error.Message;}finally{Busy=false;Downloading=false;Changed?.Invoke();}
    }
    public async Task<string> VerifiedInstaller(){if(Busy||Receipt==null)throw new InvalidOperationException("Download the update first.");return (await Run("verify","--receipt",Receipt))["path"]!.ToString();}
}

public sealed partial class MainWindow {
    UpdateClient? updater;DispatcherTimer? updateTimer;Button? updateNotice;
    void InitializeUpdates(){
        if(quick||updater!=null)return;
        try{updater=new UpdateClient(api.StatePath);updater.Changed+=()=>DispatcherQueue.TryEnqueue(()=>{if(updateNotice!=null){updateNotice.Visibility=updater.Notify?Visibility.Visible:Visibility.Collapsed;updateNotice.Content="Update available";}});
            if(Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")!=null)return;
            updateTimer=new DispatcherTimer{Interval=TimeSpan.FromMinutes(30)};updateTimer.Tick+=async(_,_)=>await updater.Check(false);updateTimer.Start();_ = updater.Check(false);
        }catch(Exception error){message.Text=error.Message;}
    }
    async Task Updates(){
        InitializeUpdates();if(updater==null)throw new InvalidOperationException("Updates could not be initialized in this build.");
        var panel=new StackPanel{Spacing=12,MaxWidth=540};var details=new TextBlock{TextWrapping=TextWrapping.Wrap};var notes=new TextBlock{TextWrapping=TextWrapping.Wrap,IsTextSelectionEnabled=true};
        var automatic=new CheckBox{Content="Check for updates automatically",IsChecked=updater.Automatic};automatic.Click+=(_,_)=>updater.Automatic=automatic.IsChecked==true;
        var preview=new CheckBox{Content="Include preview releases",IsChecked=updater.Preview};preview.Click+=async(_,_)=>{updater.Preview=preview.IsChecked==true;await updater.Check(true);};
        var check=Button("Check now",()=>updater.Check(true));var download=Button("Download update",()=>updater.Download());var cancel=Button("Cancel download",()=>{updater.Cancel();return Task.CompletedTask;});
        var installing=false;var install=Button("Install update",async()=>{if(installing)return;installing=true;foreach(var child in panel.Children.OfType<Control>())child.IsEnabled=false;try{var path=await updater.VerifiedInstaller();await api.StopLocalForUpdateAsync();Process.Start(new ProcessStartInfo(path){UseShellExecute=true});Quit();}catch(Exception error){details.Text=error.Message;try{if(api.Mode!="client")await api.StartCoordinatorAsync();}catch{/* The existing app remains available for restart. */}}finally{installing=false;foreach(var child in panel.Children.OfType<Control>())child.IsEnabled=true;}});
        var later=Button("Remind me tomorrow",()=>{updater.Later();return Task.CompletedTask;});var skip=Button("Skip this version",()=>{updater.Skip();return Task.CompletedTask;});
        var releases=Button("Release notes on GitHub",()=>{Process.Start(new ProcessStartInfo(updater.Available?["release_url"]?.ToString()??updater.Releases){UseShellExecute=true});return Task.CompletedTask;});
        void Refresh(){details.Text=updater.Message;notes.Text=updater.Available?["notes"]?.ToString()??"";check.IsEnabled=!updater.Busy&&!installing;preview.IsEnabled=!updater.Busy&&!installing;download.Visibility=updater.Available!=null&&updater.Receipt==null?Visibility.Visible:Visibility.Collapsed;download.IsEnabled=!updater.Busy&&!installing;install.Visibility=updater.Receipt!=null?Visibility.Visible:Visibility.Collapsed;install.IsEnabled=!updater.Busy&&!installing;cancel.Visibility=updater.Downloading?Visibility.Visible:Visibility.Collapsed;later.Visibility=skip.Visibility=updater.Available!=null?Visibility.Visible:Visibility.Collapsed;}
        panel.Children.Add(new TextBlock{Text="Installed: "+updater.Current});panel.Children.Add(details);panel.Children.Add(automatic);panel.Children.Add(preview);panel.Children.Add(new TextBlock{Text="Update checks contact GitHub without sending your library, queries or credentials. Installation keeps your app data and caches. Finish active file operations and pause indexing before installing.",TextWrapping=TextWrapping.Wrap,Opacity=.7});
        foreach(var element in new UIElement[]{check,download,cancel,install,later,skip,releases,new ScrollViewer{MaxHeight=180,Content=notes}})panel.Children.Add(element);
        void Changed()=>DispatcherQueue.TryEnqueue(Refresh);updater.Changed+=Changed;Refresh();try{await Dialog("Retriever updates",panel);}finally{updater.Changed-=Changed;}
    }
}
