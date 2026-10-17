using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System.Text.Json.Nodes;
using Windows.System;

namespace Retriever.Native;

public sealed partial class MainWindow {
    FileRow[] copyClipboard=[];
    Task CopyFiles(){
        var selected=(gridMode?(ListViewBase)icons:files).SelectedItems.Cast<FileRow>().ToArray();
        if(selected.Length==0)throw new InvalidOperationException("Select files to copy.");
        if(selected.Any(r=>r.Kind!="file")||selected.Select(r=>r.Device).Distinct().Count()!=1)throw new InvalidOperationException("Select regular files on one device.");
        copyClipboard=selected.Select(r=>new FileRow(r.Data.DeepClone())).ToArray();message.Text=$"{selected.Length} files ready to copy · open a destination folder and paste";return Task.CompletedTask;
    }
    Task PasteFiles(){
        if(copyClipboard.Length==0)throw new InvalidOperationException("Copy files in Retriever first.");
        if(route!="browse"||string.IsNullOrWhiteSpace(location["path"]?.ToString()))throw new InvalidOperationException("Open a destination folder before pasting.");
        return BatchPopover("copy",copyClipboard,location["device"]?.ToString(),location["path"]?.ToString());
    }
    void SetupCopyKeys(){
        foreach(var (key,action) in new (VirtualKey,Func<Task>)[]{(VirtualKey.C,CopyFiles),(VirtualKey.V,PasteFiles)}){
            var accelerator=new KeyboardAccelerator{Key=key,Modifiers=VirtualKeyModifiers.Control};
            accelerator.Invoked+=async(_,e)=>{
                if(dialogVisible||FocusManager.GetFocusedElement(root.XamlRoot) is TextBox or RichEditBox)return;
                e.Handled=true;await Guard(action);
            };root.KeyboardAccelerators.Add(accelerator);
        }
    }
    async Task<JsonNode> RunCopy(JsonNode plan,Action<JsonNode> progress){
        var id=plan["id"]?.ToString()??throw new InvalidOperationException("Missing copy plan.");
        await api.Post("workspace/start",new JsonObject{["id"]=id});
        while(true){
            var result=await api.Get("workspace/operation/"+Uri.EscapeDataString(id));
            if(result["state"]?.ToString() is not ("preview" or "running"))return result;
            progress(result["transfer"]??new JsonObject());await Task.Delay(250);
        }
    }
}
