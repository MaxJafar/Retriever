using Microsoft.UI.Xaml;
namespace Retriever.Native;
public partial class App:Application {
    MainWindow? window;
    FolderActivation? activation;
    public App(){InitializeComponent();}
    protected override async void OnLaunched(LaunchActivatedEventArgs args){
        var launch=Environment.GetCommandLineArgs().Skip(1).ToArray();
        if(launch.Contains("--restore-folder-default")){try{using var api=new Api();new FolderIntegration(new UserFolderRegistry(),Environment.ProcessPath!,api.StatePath).Restore();}catch{Environment.ExitCode=1;}Exit();return;}
        activation=new FolderActivation();if(!activation.IsOwner){try{await activation.Forward(launch);}catch{Environment.ExitCode=1;}Exit();return;}
        window=new MainWindow();window.Closed+=(_,_)=>activation.Dispose();window.Activate();
        _=activation.Listen(values=>window.DispatcherQueue.TryEnqueue(()=>window.ActivateRequest(values)));
        if(launch.Contains("--open-folder"))window.ActivateRequest(launch);else if(launch.Contains("--quick"))window.OpenQuickWhenReady();
    }
}
