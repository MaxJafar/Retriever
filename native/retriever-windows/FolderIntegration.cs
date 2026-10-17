using Microsoft.Win32;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml;
using System.Text.Json;

namespace Retriever.Native;

interface IFolderRegistry {
    string? Read(string path,string name);
    void Write(string path,string name,string? value);
}
sealed class UserFolderRegistry:IFolderRegistry {
    const string Prefix=@"Software\Classes\";
    public string? Read(string path,string name){using var key=Registry.CurrentUser.OpenSubKey(Prefix+path);var value=key?.GetValue(name,null,RegistryValueOptions.DoNotExpandEnvironmentNames);if(value!=null&&(value is not string||key!.GetValueKind(name)!=RegistryValueKind.String))throw new InvalidOperationException("An unsupported folder association already exists.");return value as string;}
    public void Write(string path,string name,string? value){if(value==null){using var existing=Registry.CurrentUser.OpenSubKey(Prefix+path,true);existing?.DeleteValue(name,false);}else{using var key=Registry.CurrentUser.CreateSubKey(Prefix+path);key.SetValue(name,value,RegistryValueKind.String);}}
}
sealed class FolderIntegration {
    public record Change(string Path,string Name,string? Before,string After);
    public record Journal(string Executable,List<Change> Changes);
    const string Verb="Retriever.Candidate.Open";
    readonly IFolderRegistry registry;readonly string executable,journal;
    public FolderIntegration(IFolderRegistry registry,string executable,string state){this.registry=registry;this.executable=Path.GetFullPath(executable);journal=Path.Combine(state,"folder-handler.json");}
    public bool IsDefault=>new[]{"Directory","Drive"}.All(type=>registry.Read(type+@"\shell","")==Verb&&registry.Read(type+@"\shell\"+Verb+@"\command","")==Command);
    string Command=>$"\"{executable}\" --open-folder \"%1\\.\"";
    public void Enable(){
        if(executable.Contains('"')||!File.Exists(executable))throw new InvalidOperationException("Install the candidate before enabling folder integration.");
        if(IsDefault)return;
        if(File.Exists(journal))throw new InvalidOperationException("Restore the previous integration before enabling it again.");
        var changes=new List<Change>();
        foreach(var type in new[]{"Directory","Drive"}){
            var shell=type+@"\shell";var verb=shell+"\\"+Verb;
            if(registry.Read(verb+@"\command","")!=null)throw new InvalidOperationException("Another Retriever build owns this folder integration. Restore it there first.");
            foreach(var value in new[]{(verb,"","Open in Retriever"),(verb,"Icon",executable),(verb+@"\command","",Command),(shell,"",Verb)})changes.Add(new(value.Item1,value.Item2,registry.Read(value.Item1,value.Item2),value.Item3));
        }
        Directory.CreateDirectory(Path.GetDirectoryName(journal)!);File.WriteAllText(journal+".tmp",JsonSerializer.Serialize(new Journal(executable,changes)));File.Move(journal+".tmp",journal);
        try{foreach(var change in changes)registry.Write(change.Path,change.Name,change.After);}catch{Restore();throw;}
    }
    public void Restore(){
        if(!File.Exists(journal))return;
        if(new FileInfo(journal).Length>32*1024)throw new InvalidOperationException("Folder integration backup is invalid.");
        var saved=JsonSerializer.Deserialize<Journal>(File.ReadAllText(journal))??throw new InvalidOperationException("Missing folder integration backup.");
        if(!string.Equals(saved.Executable,executable,StringComparison.OrdinalIgnoreCase))throw new InvalidOperationException("Restore folder integration from the build that enabled it.");
        // Validate the complete journal before touching any registry value.
        foreach(var change in saved.Changes){if(!new[]{"Directory","Drive"}.Any(t=>change.Path==t+@"\shell"||change.Path==t+@"\shell\"+Verb||change.Path==t+@"\shell\"+Verb+@"\command")||!new[]{"","Icon"}.Contains(change.Name))throw new InvalidOperationException("Invalid folder integration backup.");}
        foreach(var type in new[]{"Directory","Drive"}){
            var command=registry.Read(type+@"\shell\"+Verb+@"\command","");
            if(command!=null&&command!=Command)continue;
            foreach(var change in saved.Changes.Where(v=>v.Path.StartsWith(type+"\\",StringComparison.Ordinal)).Reverse())if(registry.Read(change.Path,change.Name)==change.After)registry.Write(change.Path,change.Name,change.Before);
        }
        File.Delete(journal);
    }
}
public sealed partial class MainWindow {
    Task FolderIntegrationPopover(){
        var integration=new FolderIntegration(new UserFolderRegistry(),Environment.ProcessPath!,api.StatePath);var body=new StackPanel{Spacing=12};
        body.Children.Add(new TextBlock{Text="Open folders in Retriever",FontSize=17,FontWeight=Microsoft.UI.Text.FontWeights.SemiBold});
        body.Children.Add(new TextBlock{Text="Use Retriever for ordinary folder and drive double-clicks in this Windows account. Existing Explorer windows, Win+E, desktop icons and system file dialogs remain managed by Windows.",TextWrapping=TextWrapping.Wrap,FontSize=12,Opacity=.75});
        var feedback=new TextBlock{Text=integration.IsDefault?"Retriever is selected for folders.":"Windows folder behavior is unchanged.",TextWrapping=TextWrapping.Wrap,FontSize=11};body.Children.Add(feedback);
        var enable=FlatButton("Use Retriever for folders","\uE8B7",()=>{integration.Enable();feedback.Text="Enabled. Test a folder, then restore here at any time.";return Task.CompletedTask;});
        var restore=FlatButton("Restore previous folder behavior","\uE777",()=>{integration.Restore();feedback.Text="Previous settings restored; newer choices made by other apps are preserved.";return Task.CompletedTask;});
#if RETRIEVER_TESTING
        enable.IsEnabled=false;restore.IsEnabled=false;feedback.Text="Fixture preview · OS associations are never changed.";
#else
        enable.IsEnabled=api.Mode!="client"&&Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")==null;
#endif
        body.Children.Add(enable);body.Children.Add(restore);body.Children.Add(new TextBlock{Text="Retriever stores a rollback record before changing this account's folder actions. This is an optional integration, not a Windows shell replacement.",FontSize=10,Opacity=.6,TextWrapping=TextWrapping.Wrap});
        Popover("Folder integration",body,width:340);return Task.CompletedTask;
    }
}
