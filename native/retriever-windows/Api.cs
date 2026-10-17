using System.Diagnostics;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json.Nodes;
using Windows.Security.Cryptography;
using Windows.Security.Cryptography.DataProtection;
namespace Retriever.Native;

public sealed class Api:IDisposable {
#if RETRIEVER_TESTING
    internal HealthFixtures? HealthTestProvider;
    internal JsonNode? CapacityTestFixture;
#endif
    readonly HttpClient client=new(new HttpClientHandler{AllowAutoRedirect=false}){Timeout=TimeSpan.FromSeconds(125)};
    readonly string coordinatorEndpoint;
    readonly bool preferCuda;
    readonly JsonObject? sharedRelaySettings;
    SharedLibraryRelay? sharedRelay;
    public Api(){
        var profile=Environment.GetEnvironmentVariable("RETRIEVER_PROFILE");
        var savedInstallation=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"Retriever Candidate","installation.json");
        var bootstrap=File.Exists(savedInstallation)?savedInstallation:Path.Combine(AppContext.BaseDirectory,"coordinator.json");
        if(profile==null&&File.Exists(bootstrap)){
            // Adopt migrated data in place, including redirected application
            // data, without copying or clearing the user's metadata or caches.
            var value=JsonNode.Parse(File.ReadAllText(bootstrap))??throw new InvalidOperationException("Invalid coordinator location.");
            var state=value["state"]?.ToString()??throw new InvalidOperationException("Missing coordinator state location.");
            if(!Path.IsPathFullyQualified(state))throw new InvalidOperationException("The coordinator state location must be absolute.");
            StatePath=Path.GetFullPath(state);
            var configuredPort=value["port"]?.GetValue<int>()??18780;
            if(configuredPort<1024||configuredPort>65535)throw new InvalidOperationException("Invalid coordinator port.");
            Endpoint=$"http://127.0.0.1:{configuredPort}";
            preferCuda=value["accelerator"]?.ToString()=="cuda";
            sharedRelaySettings=value["shared_relay"]?.DeepClone() as JsonObject;
            if(bootstrap!=savedInstallation){Directory.CreateDirectory(Path.GetDirectoryName(savedInstallation)!);var temporary=savedInstallation+"."+Guid.NewGuid().ToString("N");File.WriteAllText(temporary,value.ToJsonString());File.Move(temporary,savedInstallation,false);}
        }
        if(profile!=null&&System.Text.RegularExpressions.Regex.IsMatch(profile,@"^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$"))StatePath=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"Retriever Candidate",profile);
        if(int.TryParse(Environment.GetEnvironmentVariable("RETRIEVER_PORT"),out var port)&&port>=1024&&port<=65535)Endpoint=$"http://127.0.0.1:{port}";
        coordinatorEndpoint=Endpoint;
    }
    public string Endpoint{get;private set;}="http://127.0.0.1:18780";
    public string StatePath{get;private set;}=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"Retriever Candidate","state");
    public string Mode{get;private set;}="coordinator";
    public string Token{get;private set;}="";
    public Process? OwnedService{get;private set;}
    static string ExpectedEngineVersion=>System.Reflection.Assembly.GetExecutingAssembly().GetCustomAttributes(typeof(System.Reflection.AssemblyInformationalVersionAttribute),false).Cast<System.Reflection.AssemblyInformationalVersionAttribute>().Single().InformationalVersion.Split('+')[0];
    static void RequireCompatible(JsonNode status){
        if(status["runtime"]?.ToString()!="rust")throw new InvalidOperationException("This endpoint belongs to another engine.");
        if(status["version"]?.ToString()!=ExpectedEngineVersion&&status["desktop_api_version"]?.GetValue<int>()!=1&&status["version"]?.ToString()!="0.13.0-alpha.20-native")throw new InvalidOperationException("This coordinator uses a different desktop protocol. Update the coordinator or use a compatible Retriever client.");
    }
    string Preferences=>Path.Combine(Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")==null?Path.GetDirectoryName(StatePath)!:StatePath,"connection.json");
    public void Connect(string endpoint,string token){var uri=new Uri(endpoint);if(uri.UserInfo.Length>0||uri.Query.Length>0||uri.Fragment.Length>0||(uri.Scheme!="https"&&!(uri.Scheme=="http"&&uri.IsLoopback)))throw new ArgumentException("Use HTTPS or a loopback tunnel without credentials in the URL.");Endpoint=endpoint.TrimEnd('/');Token=token.Trim();client.DefaultRequestHeaders.Authorization=new AuthenticationHeaderValue("Bearer",Token);}
    public async Task StartAsync(){
#if RETRIEVER_TESTING
        if(Environment.GetEnvironmentVariable("RETRIEVER_FIXTURE_RESUME")=="1"){
            if(Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")?.StartsWith("workspace-smoke-health-offline-")!=true)throw new InvalidOperationException("Fixture resume requires an isolated profile.");
            var fixture=Environment.GetEnvironmentVariable("RETRIEVER_HEALTH_FIXTURES")??throw new InvalidOperationException("Missing health fixtures");
            HealthTestProvider=new HealthFixtures(fixture);
            CapacityTestFixture=JsonNode.Parse(await File.ReadAllTextAsync(Path.Combine(Path.GetDirectoryName(fixture)!,"disk-capacity.json")));
        }
#endif
        if(Environment.GetEnvironmentVariable("RETRIEVER_PROFILE")==null&&File.Exists(Preferences)){var preferences=JsonNode.Parse(await File.ReadAllTextAsync(Preferences))!;Mode=preferences["mode"]?.ToString()??"coordinator";
            if(Mode=="client"){var protectedToken=CryptographicBuffer.DecodeFromBase64String(preferences["credential"]!.ToString());var plaintext=await new DataProtectionProvider().UnprotectAsync(protectedToken);Connect(preferences["endpoint"]!.ToString(),CryptographicBuffer.ConvertBinaryToString(BinaryStringEncoding.Utf8,plaintext));return;}}
        await StartCoordinatorAsync();
        if(sharedRelaySettings!=null)sharedRelay??=new SharedLibraryRelay(sharedRelaySettings,StatePath,new Uri(Endpoint).Port);
    }
    public async Task SaveClientAsync(string endpoint,string token){
        var previous=(Endpoint,Token,Mode);try{Connect(endpoint,token);RequireCompatible(await Get("status"));var data=CryptographicBuffer.ConvertStringToBinary(Token,BinaryStringEncoding.Utf8);var protectedToken=await new DataProtectionProvider("LOCAL=user").ProtectAsync(data);await SavePreferences(new JsonObject{["mode"]="client",["endpoint"]=Endpoint,["credential"]=CryptographicBuffer.EncodeToBase64String(protectedToken)});Mode="client";}catch{Connect(previous.Endpoint,previous.Token);Mode=previous.Mode;throw;}
    }
    public async Task UseCoordinatorAsync(){
        var previous=(Endpoint,Token,Mode);try{Mode="coordinator";Endpoint=coordinatorEndpoint;await StartCoordinatorAsync();await SavePreferences(new JsonObject{["mode"]=Mode});}catch{Connect(previous.Endpoint,previous.Token);Mode=previous.Mode;throw;}
    }
    async Task SavePreferences(JsonNode data){Directory.CreateDirectory(Path.GetDirectoryName(Preferences)!);var temporary=Preferences+"."+Guid.NewGuid().ToString("N");await File.WriteAllTextAsync(temporary,data.ToJsonString());File.Move(temporary,Preferences,true);}
    public async Task StartCoordinatorAsync(){
        Directory.CreateDirectory(StatePath);
        if(File.Exists(Path.Combine(StatePath,"index.sqlite")))throw new InvalidOperationException("Choose separate candidate state; the installed index is protected.");
        var exe=Path.Combine(AppContext.BaseDirectory,"engine","retriever.exe");
        if(!File.Exists(Path.Combine(StatePath,"api-token"))){if(!File.Exists(exe))throw new FileNotFoundException("The bundled Rust engine is missing.");using var init=Process.Start(new ProcessStartInfo(exe){UseShellExecute=false,CreateNoWindow=true,ArgumentList={"--state",StatePath,"init"}})!;await init.WaitForExitAsync();if(init.ExitCode!=0)throw new InvalidOperationException("Candidate state could not be initialized.");}
        Connect(Endpoint,await File.ReadAllTextAsync(Path.Combine(StatePath,"api-token")));
        try{var status=await Get("status");RequireCompatible(status);return;}
        catch(HttpRequestException){ }
        if(!File.Exists(exe))throw new FileNotFoundException("The bundled Rust engine is missing.");
        var launches=new List<ProcessStartInfo>();
        ProcessStartInfo Launch(string binary)=>new(binary){UseShellExecute=false,CreateNoWindow=true,ArgumentList={"--state",StatePath,"serve","--port",new Uri(Endpoint).Port.ToString()}};
        // Acceleration is an opt-in for a validated local deployment. The
        // portable CPU engine remains available when its toolkit is absent.
        var accelerated=Path.Combine(AppContext.BaseDirectory,"engine","retriever-cuda.exe");
        var toolkit=Environment.GetEnvironmentVariable("CUDA_PATH");
        if(preferCuda&&File.Exists(accelerated)&&toolkit!=null&&Path.IsPathFullyQualified(toolkit)){
            var runtime=new[]{Path.Combine(toolkit,"bin","x64"),Path.Combine(toolkit,"bin")}.FirstOrDefault(folder=>new[]{"cudart64_13.dll","cublas64_13.dll","cublasLt64_13.dll"}.All(name=>File.Exists(Path.Combine(folder,name))));
            if(runtime!=null){var launch=Launch(accelerated);launch.Environment.TryGetValue("PATH",out var inheritedPath);launch.Environment["PATH"]=runtime+Path.PathSeparator+inheritedPath;launches.Add(launch);}
        }
        launches.Add(Launch(exe));
        foreach(var launch in launches){
            try{OwnedService=Process.Start(launch);}catch(System.ComponentModel.Win32Exception) when(launch.FileName!=exe){continue;}
            for(int i=0;i<1200;i++){await Task.Delay(100);try{var s=await Get("status");RequireCompatible(s);return;}catch(HttpRequestException){} if(OwnedService?.HasExited==true)break;}
            // Do not create a second writer while a slow service is still alive.
            if(OwnedService?.HasExited==false)break;
        }
        throw new InvalidOperationException($"The Rust coordinator did not become ready. Check the state folder and port {new Uri(Endpoint).Port}.");
    }
    public Task<JsonNode> Get(string route,CancellationToken cancel=default)=>Send(HttpMethod.Get,route,null,null,cancel);
    public Task<JsonNode> Post(string route,JsonNode? body=null,string? revision=null,CancellationToken cancel=default)=>Send(HttpMethod.Post,route,body??new JsonObject(),revision,cancel);
    async Task<JsonNode> Send(HttpMethod method,string route,JsonNode? body,string? revision,CancellationToken cancel){
#if RETRIEVER_TESTING
        if(CapacityTestFixture!=null&&(route=="disk"||route.StartsWith("disk/",StringComparison.Ordinal)))return CapacityFixtures.Request(CapacityTestFixture,route,body);
        if(route=="health"||route.StartsWith("health/",StringComparison.Ordinal))return (HealthTestProvider??throw new InvalidOperationException("Health fixtures must be injected before any test request")).Request(route,body);
#endif
        using var request=new HttpRequestMessage(method,Endpoint+"/api/"+route);if(body!=null)request.Content=new StringContent(body.ToJsonString(),Encoding.UTF8,"application/json");if(revision!=null)request.Headers.TryAddWithoutValidation("If-Match",revision);
        using var response=await client.SendAsync(request,cancel);var value=JsonNode.Parse(await response.Content.ReadAsStringAsync(cancel))??new JsonObject();if(!response.IsSuccessStatusCode)throw new InvalidOperationException(value["error"]?.ToString()??response.StatusCode.ToString());return value;
    }
    public void SetClientMode()=>Mode="client";
    public async Task StopLocalForUpdateAsync(){
        if(Mode=="client")return;
        using var local=new HttpClient(new HttpClientHandler{AllowAutoRedirect=false}){Timeout=TimeSpan.FromSeconds(20)};
        local.DefaultRequestHeaders.Authorization=new AuthenticationHeaderValue("Bearer",(await File.ReadAllTextAsync(Path.Combine(StatePath,"api-token"))).Trim());
        var running=JsonNode.Parse(await local.GetStringAsync(coordinatorEndpoint+"/api/status"))!;
        if(running["update_protocol"]?.GetValue<int>()!=1)throw new InvalidOperationException("This older engine needs a manual restart for its first update. Finish active work, close its service, then run the downloaded installer.");
        local.DefaultRequestHeaders.Add("X-Retriever-Update-Token",(await File.ReadAllTextAsync(Path.Combine(StatePath,"update-token"))).Trim());
        using var response=await local.PostAsync(coordinatorEndpoint+"/api/runtime/shutdown",new StringContent(new JsonObject{["expected_version"]=running["version"]?.DeepClone()}.ToJsonString(),Encoding.UTF8,"application/json"));
        var value=JsonNode.Parse(await response.Content.ReadAsStringAsync())!;
        if(!response.IsSuccessStatusCode)throw new InvalidOperationException(value["error"]?.ToString()??"The engine could not prepare for the update.");
        var deadline=DateTime.UtcNow.AddSeconds(15);
        while(DateTime.UtcNow<deadline){await Task.Delay(100);using var timeout=new CancellationTokenSource(TimeSpan.FromSeconds(1));try{using var probe=await local.GetAsync(coordinatorEndpoint+"/api/status",timeout.Token);}catch(HttpRequestException){return;}catch(OperationCanceledException){}}
        throw new InvalidOperationException("The local engine is still stopping. Wait before running the installer.");
    }
    public void Dispose(){sharedRelay?.Dispose();client.Dispose();} // The indexing service remains reusable after closing the desktop.
}
public sealed class FileRow:System.ComponentModel.INotifyPropertyChanged {
    public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;
    public Microsoft.UI.Xaml.Media.ImageSource? Icon{get;private set;}
    public JsonNode Data{get;}
    public FileRow(JsonNode node,string? deviceLabel=null){Data=node;DeviceLabel=deviceLabel??Device;_ = LoadIcon();}
    public string DeviceLabel{get;}
    public string Modified=>double.TryParse(Data["modified"]?.ToString(),out var seconds)&&seconds>0?DateTimeOffset.FromUnixTimeSeconds((long)seconds).ToLocalTime().ToString("MM/dd/yyyy, HH:mm"):"—";
    public string TypeLabel=>Kind=="folder"?"File folder":System.IO.Path.GetExtension(Path).TrimStart('.').ToLowerInvariant() switch {"pdf"=>"PDF document","docx"=>"Word document","xlsx"=>"Excel worksheet","pptx"=>"PowerPoint presentation","md"=>"Markdown document","txt" or "log"=>"Text document","png" or "jpg" or "jpeg"=>"Image","mp4" or "mkv" or "mov"=>"Video",var ext=>string.IsNullOrEmpty(ext)?"File":ext.ToUpperInvariant()+" file"};
    async Task LoadIcon(){try{var key=Kind=="folder"?"folder":System.IO.Path.GetExtension(Path).ToLowerInvariant();if(string.IsNullOrEmpty(key))key="file";var value=System.Text.Json.JsonSerializer.SerializeToNode(await FileIcons.Read([key]));var src=value?["icons"]?[key]?["src"]?.ToString();if(src==null)return;var bytes=Convert.FromBase64String(src[(src.IndexOf(',')+1)..]);using var stream=new Windows.Storage.Streams.InMemoryRandomAccessStream();using(var writer=new Windows.Storage.Streams.DataWriter(stream)){writer.WriteBytes(bytes);await writer.StoreAsync();writer.DetachStream();}stream.Seek(0);var image=new Microsoft.UI.Xaml.Media.Imaging.BitmapImage();await image.SetSourceAsync(stream);Icon=image;PropertyChanged?.Invoke(this,new(nameof(Icon)));}catch{ /* The native glyph remains available if an association has no icon. */ }}
    public string Name=>Data["name"]?.ToString()??"";
    public string Path=>Data["path"]?.ToString()??"";
    public string ParentPath {get {var split=Path.LastIndexOfAny(['/','\\']);return split<0?Path:Path[..(split==0?1:split==2&&Path[1]==':'?3:split)];}}
    public string Device=>Data["device"]?.ToString()??"";
    public string Kind=>Data["kind"]?.ToString()??"file";
    public string Type=>Data["type"]?.ToString()??"";
    public string Detail {
        get {
            var snippet=Data["snippet"]?.ToString();
            var detail=Data["detail"]?.ToString()??"";
            var offline=Data["source_status"]?.ToString()=="offline"?"Offline · cached result · ":"";
            if(!string.IsNullOrWhiteSpace(snippet))return offline+(Data["generated"]?.GetValue<bool>()==true?"AI description · ":"")+System.Text.RegularExpressions.Regex.Replace(snippet,@"\s+"," ").Trim();
            if(detail.Contains("pdf_open_failed")||detail.Contains("extraction_failed"))return offline+"Content could not be read · filename search available";
            if(detail.Contains("scanned_pdf"))return offline+"Scanned PDF · no extracted text";
            return offline+(Kind=="folder"?"Folder":System.IO.Path.GetExtension(Path).TrimStart('.').ToUpperInvariant()+" file");
        }
    }
    public string Size=>long.TryParse(Data["size"]?.ToString(),out var n)?n<1024?$"{n} B":n<1048576?$"{n/1024d:N1} KB":$"{n/1048576d:N1} MB":"";
    public string Priority=>Data["metadata"]?["priority"]?.ToString()??"";
    bool selected;
    public bool IsSelected{get=>selected;set{if(selected==value)return;selected=value;PropertyChanged?.Invoke(this,new(nameof(IsSelected)));}}
    public double? DurationSeconds=>double.TryParse(Data["duration_seconds"]?.ToString(),out var value)&&double.IsFinite(value)&&value>0?value:null;
    public string Duration=>DurationSeconds is double value?FormatDuration(value):"—";
    public void SetDuration(JsonNode? value){Data["duration_seconds"]=value?.DeepClone();PropertyChanged?.Invoke(this,new(nameof(Duration)));PropertyChanged?.Invoke(this,new(nameof(DurationSeconds)));}
    public static string FormatDuration(double value){var span=TimeSpan.FromSeconds(Math.Max(0,value));return span.TotalHours>=1?$"{(int)span.TotalHours}:{span.Minutes:00}:{span.Seconds:00}":$"{span.Minutes}:{span.Seconds:00}";}
    public JsonNode Metadata=>Data["metadata"]??new JsonObject();
    public string Tags=>string.Join(" · ",(Data["metadata"]?["tags"] as JsonArray??[]).Select(t=>t?.ToString()));
    public string Status=>Data["metadata"]?["status"]?.ToString()??"";
    public Dictionary<string,string> PropertyLabels=>(Data["metadata"]?["properties"] as JsonObject??new()).ToDictionary(p=>p.Key,p=>p.Value is JsonArray a?string.Join(" · ",a.Select(v=>v?.ToString())):p.Value?.ToString()??"—");
    public void SetMetadata(JsonNode metadata){Data["metadata"]=metadata.DeepClone();PropertyChanged?.Invoke(this,new(null));}
    public string Glyph=>Kind=="folder"?"\uE8B7":"\uE8A5";
}
