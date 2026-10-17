using System.Text.Json;
using Retriever.Native;

var root=Path.GetFullPath(args.Single());
if(Directory.Exists(root)&&Directory.EnumerateFileSystemEntries(root).Any())throw new Exception("Test output must be empty");
Directory.CreateDirectory(root);
var cases=0;
void Require(bool value,string detail){if(!value)throw new Exception(detail);}
void Rejected(Action action){try{action();}catch(InvalidOperationException){return;}throw new Exception("Expected rejection");}
void Test(string name,Action<Fixture> action){var test=new Fixture(Path.Combine(root,name));action(test);cases++;Console.WriteLine("PASS "+name);}
Test("no-opt-in",f=>Require(!f.New.UpgradeOwned()&&f.Registry.Values.Count==1,"Changed an unregistered handler"));
Test("upgrade-and-restore",f=>{
    f.Old.Enable();Require(f.New.UpgradeOwned()&&f.New.IsDefault,"Upgrade did not retarget both handlers");
    Rejected(f.Old.Restore);Require(f.New.IsDefault,"Old uninstaller removed the new handler");
    f.New.Restore();Require(f.Registry.Read(@"Directory\shell","")=="Original"&&f.Registry.Read(@"Drive\shell","")==null,"Restore lost the original selection");
});
Test("preserve-newer-choice",f=>{
    f.Old.Enable();f.Registry.Write(@"Directory\shell","","AnotherApp");
    f.Registry.Write(@"Directory\shell\Retriever.Candidate.Open\command","","third-party-command");
    f.New.UpgradeOwned();f.New.Restore();
    Require(f.Registry.Read(@"Directory\shell","")=="AnotherApp"&&f.Registry.Read(@"Directory\shell\Retriever.Candidate.Open\command","")=="third-party-command","Overwrote a newer choice");
});
Test("rollback-on-write-error",f=>{
    f.Old.Enable();var journal=File.ReadAllBytes(f.Journal);var values=new Dictionary<string,string>(f.Registry.Values);
    f.Registry.FailIn=2;Rejected(()=>f.New.UpgradeOwned());
    Require(File.ReadAllBytes(f.Journal).SequenceEqual(journal)&&values.OrderBy(p=>p.Key).SequenceEqual(f.Registry.Values.OrderBy(p=>p.Key)),"Failed handoff lost the old registration");
    Require(!Directory.EnumerateFiles(f.State,"*.tmp").Any(),"Temporary journal was retained");
    Require(f.New.UpgradeOwned()&&f.New.IsDefault,"Retry failed");
});
Test("interrupted-handoff",f=>{
    f.Old.Enable();f.Registry.Write(@"Directory\shell\Retriever.Candidate.Open\command","",$"\"{f.NewPath}\" --open-folder \"%1\\.\"");
    Require(f.New.UpgradeOwned()&&f.New.IsDefault,"Interrupted handoff could not recover");
    f.New.Restore();Require(f.Registry.Read(@"Directory\shell","")=="Original","Recovered restore lost the original choice");
});
Test("tampered-journal",f=>{
    f.Old.Enable();var saved=JsonSerializer.Deserialize<FolderIntegration.Journal>(File.ReadAllText(f.Journal))!;
    saved.Changes[0]=saved.Changes[0] with{Path=@"Unrelated\Setting"};File.WriteAllText(f.Journal,JsonSerializer.Serialize(saved));
    var count=f.Registry.Writes;Rejected(()=>f.New.UpgradeOwned());Require(f.Registry.Writes==count,"Invalid journal changed registry values");
});
Test("missing-replacement",f=>{
    f.Old.Enable();var missing=new FolderIntegration(f.Registry,Path.Combine(f.State,"missing.exe"),f.State);
    var count=f.Registry.Writes;Rejected(()=>missing.UpgradeOwned());Require(f.Registry.Writes==count&&f.Old.IsDefault,"Missing replacement changed registration");
});
Console.WriteLine(JsonSerializer.Serialize(new{passed=true,cases,registry="in-memory",system_associations_changed=false}));

sealed class MemoryRegistry:IFolderRegistry {
    public Dictionary<string,string> Values=new();public int FailIn, Writes;
    public string? Read(string path,string name)=>Values.GetValueOrDefault(path+"|"+name);
    public void Write(string path,string name,string? value){
        if(FailIn>0&&--FailIn==0)throw new InvalidOperationException("Injected write failure");
        Writes++;if(value==null)Values.Remove(path+"|"+name);else Values[path+"|"+name]=value;
    }
}
sealed class Fixture {
    public readonly string State,Journal,NewPath;public readonly MemoryRegistry Registry=new();public readonly FolderIntegration Old,New;
    public Fixture(string path){
        State=path;Directory.CreateDirectory(path);var old=Path.Combine(path,"old.exe");NewPath=Path.Combine(path,"new.exe");
        File.WriteAllText(old,"fixture only");File.WriteAllText(NewPath,"fixture only");Journal=Path.Combine(path,"folder-handler.json");
        Registry.Write(@"Directory\shell","","Original");Old=new(Registry,old,path);New=new(Registry,NewPath,path);
    }
}
