#if RETRIEVER_TESTING
using System.Text.Json.Nodes;
namespace Retriever.Native;
sealed class MemoryFolderRegistry:IFolderRegistry {
    public readonly Dictionary<string,string> Values=new();
    public string? Read(string path,string name)=>Values.GetValueOrDefault(path+"|"+name);
    public void Write(string path,string name,string? value){if(value==null)Values.Remove(path+"|"+name);else Values[path+"|"+name]=value;}
}
public sealed partial class MainWindow {
    async Task TestExplorerFoundation(Action<bool,string> require){
        var fixtures=Environment.GetEnvironmentVariable("RETRIEVER_FIXTURES")!;var device=status!["local_device"]!.ToString();var child=Path.Combine(fixtures,"Explorer tree","Nested folder");Directory.CreateDirectory(child);File.WriteAllText(Path.Combine(child,"example.txt"),"A safe folder tree fixture");
        await BuildFolderTree();var key=device+":"+fixtures;expandedFolders.Add(key);await LoadFolderBranch(device,fixtures,true);require(folderBranches[key].Entries.Any(v=>v?["name"]?.ToString()=="Explorer tree"),"Folder expansion did not list the fixture directory");
        require(folderBranches[key].Entries.All(v=>v?["kind"]?.ToString()=="folder"),"Sidebar included a regular file");SaveFolderTree();expandedFolders.Clear();treeEndpoint="";RestoreFolderTree();require(expandedFolders.Contains(key),"Folder expansion was not restored");
        await Browse(device,child);require(rows.Count==1&&rows[0].Name=="example.txt","Tree folder navigation did not list its file");
        var revisionBefore=(await api.Get("config"))["revision"]!.ToString();await ReviewCrawlScope(child);await Task.Delay(150);require(activePopover!=null,"Crawl review popover did not open");require((await api.Get("config"))["revision"]!.ToString()==revisionBefore,"Crawl review changed the configured scope");await RenderWorkspaceTest((Microsoft.UI.Xaml.UIElement)activePopover!.Content,"explorer-setup.png");
        var plan=await api.Post("crawl/preview",new JsonObject{["device"]=device,["root"]=child});require(plan["crawling_started"]?.GetValue<bool>()==false&&plan["files_read"]?.GetValue<int>()==0,"Crawl review read source contents");
        TestInvoke(TestButtons((Microsoft.UI.Xaml.DependencyObject)activePopover!.Content).First(b=>TestButtonText(b)=="Save & start local crawl"));
        bool saved=false;for(int i=0;i<100;i++){await Task.Delay(75);var cfg=await api.Get("config");saved=cfg["config"]?["devices"]?.AsArray().Any(d=>d?["id"]?.ToString()==device&&d["roots"]!.AsArray().Any(p=>p?.ToString()==child))==true;if(saved&&(await api.Get("status"))["syncing"]?.GetValue<bool>()!=true)break;}
        require(saved,"Onboarding save button did not include the chosen folder");
        var folder=(await api.Get("crawl"))["folders"]!.AsArray().First(f=>f?["root"]?.ToString()==child)!;require(folder["interval_minutes"]?.GetValue<int>()==0&&folder["analysis"]?.ToString()=="off","Onboarding unexpectedly enabled a schedule or model analysis");
        await RefreshConfig();await ClosePopover();
        var registry=new MemoryFolderRegistry();registry.Write(@"Directory\shell","","ExistingChoice");var backup=Path.Combine(api.StatePath,"association-fixture");Directory.CreateDirectory(backup);
        var integration=new FolderIntegration(registry,Environment.ProcessPath!,backup);integration.Enable();require(integration.IsDefault,"Folder integration did not set both folder and drive verbs");require(registry.Read(@"Directory\shell\Retriever.Candidate.Open\command","")!.Contains("--open-folder \"%1\\.\""),"Folder path was not quoted");integration.Restore();require(registry.Read(@"Directory\shell","")=="ExistingChoice"&&registry.Read(@"Drive\shell","")==null,"Folder integration rollback lost prior/absent values");
        integration.Enable();registry.Write(@"Directory\shell","","NewerChoice");integration.Restore();require(registry.Read(@"Directory\shell","")=="NewerChoice","Rollback overwrote a newer user choice");
        require(FolderActivation.Folder(["--open-folder",child])==Path.GetFullPath(child),"Folder activation lost spaces in a path");bool rejected=false;try{FolderActivation.Folder(["--open-folder","relative"]);}catch(ArgumentException){rejected=true;}require(rejected,"Relative shell activation was accepted");
        await FolderIntegrationPopover();await Task.Delay(150);require(activePopover!=null,"Folder integration popover missing");await ClosePopover();
        await Navigate("files");await RenderWorkspaceTest(root,"explorer-tree.png");
    }
}
#endif
