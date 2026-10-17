using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    readonly System.Runtime.CompilerServices.ConditionalWeakTable<CheckBox,object> selectionHooks=new();
    Button? batchActions;
    readonly CheckBox selectPage=new(){Width=24,MinWidth=24,MinHeight=24,Padding=new Thickness(0),HorizontalAlignment=HorizontalAlignment.Left,VerticalAlignment=VerticalAlignment.Center};
    bool updatingSelection;
    bool sumLengths=true;
    async Task ReadVisibleLengths(){
        var generation=querySequence;foreach(var row in rows.Where(r=>r.Type=="video"&&!r.DurationSeconds.HasValue).Take(20).ToArray()){
            if(generation!=querySequence)return;try{var data=await api.Post("explorer/duration",new JsonObject{["device"]=row.Device,["path"]=row.Path});if(generation!=querySequence)return;row.SetDuration(data["duration_seconds"]);UpdatePager();}catch(Exception){/* Unknown is retained if a decoder is unavailable or busy. */}
        }
    }
    void WireSelectionCells(DependencyObject parent){
        if(parent is CheckBox check&&check.Tag?.ToString()=="select-file"&&!selectionHooks.TryGetValue(check,out _)){
            selectionHooks.Add(check,new());check.Click+=(_,_)=>{if(check.DataContext is FileRow row){if(check.IsChecked==true&&!files.SelectedItems.Contains(row))files.SelectedItems.Add(row);else if(check.IsChecked!=true)files.SelectedItems.Remove(row);UpdateFileSelection();}};
        }
        for(int i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++)WireSelectionCells(VisualTreeHelper.GetChild(parent,i));
    }
    void SetupFileSelection(){
        SetupCopyKeys();
        foreach(var list in new ListViewBase[]{files,icons})list.SelectionChanged+=(_,_)=>UpdateFileSelection();
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(selectPage,"Select all files on this page");
        selectPage.Click+=(_,_)=>{var view=gridMode?(ListViewBase)icons:files;updatingSelection=true;try{if(selectPage.IsChecked==true){foreach(var row in rows)if(!view.SelectedItems.Contains(row))view.SelectedItems.Add(row);}else view.SelectedItems.Clear();}finally{updatingSelection=false;UpdateFileSelection();}};
    }
    void UpdateFileSelection(){
        if(updatingSelection)return;var view=gridMode?(ListViewBase)icons:files;foreach(var row in rows)row.IsSelected=view.SelectedItems.Contains(row);
        selectPage.IsChecked=rows.Count>0&&view.SelectedItems.Count==rows.Count;
        if(batchActions!=null){batchActions.IsEnabled=view.SelectedItems.Count>0;if(batchActions.Content is StackPanel panel&&panel.Children.Last() is TextBlock label)label.Text=view.SelectedItems.Count>0?$"{view.SelectedItems.Count} selected":"Select files";}
        if(previousPage!=null)UpdatePager();
    }
    string SelectionSummary(){var view=gridMode?(ListViewBase)icons:files;var selected=view.SelectedItems.Cast<FileRow>().ToArray();var source=selected.Length>0?selected:rows.ToArray();var known=source.Where(r=>r.DurationSeconds.HasValue).ToArray();return (selected.Length>0?$" · {selected.Length} selected":"")+(sumLengths&&known.Length>0?$" · Σ {FileRow.FormatDuration(known.Sum(r=>r.DurationSeconds!.Value))} ({known.Length} measured · {(selected.Length>0?"selection":"this page")})":"");}
    Task BatchMenu(){
        var body=new StackPanel{Spacing=5};foreach(var (op,label,icon) in new[]{("rename","Rename selected","\uE8AC"),("move","Move selected","\uE8DE"),("copy","Copy selected","\uE8C8"),("trash","Move to Recycle Bin","\uE74D")})body.Children.Add(FlatButton(label,icon,async()=>{await ClosePopover();await BatchPopover(op);}));
        Popover("Selected files",body,batchActions,270);return Task.CompletedTask;
    }
    Task BatchPopover(string operation,FileRow[]? sourceFiles=null,string? targetDeviceId=null,string? destination=null){
        var selected=sourceFiles??(gridMode?(ListViewBase)icons:files).SelectedItems.Cast<FileRow>().ToArray();if(selected.Length==0)throw new InvalidOperationException("Select files with the checkboxes first.");if(selected.Select(r=>r.Device).Distinct().Count()!=1)throw new InvalidOperationException("Choose files on one source device at a time.");
        var remoteTrash=operation=="trash"&&(api.Mode=="client"||selected[0].Device!=status?["local_device"]?.ToString());
        var body=new StackPanel{Spacing=10};body.Children.Add(Label($"{selected.Length} selected",13));
        var target=new ComboBox{Header="Destination device",HorizontalAlignment=HorizontalAlignment.Stretch};
        foreach(var node in config?["devices"]?.AsArray()??[]){if(node==null||node["enabled"]?.GetValue<bool>()==false)continue;var option=new ComboBoxItem{Content=node["label"]?.ToString(),Tag=node["id"]?.ToString()};target.Items.Add(option);if(option.Tag?.ToString()==(targetDeviceId??selected[0].Device))target.SelectedItem=option;}
        if(operation=="copy")body.Children.Add(target);
        var input=new TextBox{Header=operation=="rename"?"Name pattern · {name}, {n}, {ext}":"Destination folder",Text=operation=="rename"?"{name} ({n}){ext}":destination??location["path"]?.ToString()??""};if(operation!="trash")body.Children.Add(input);
        target.SelectionChanged+=async(_,_)=>{if(operation!="copy")return;input.Text="";try{var id=(target.SelectedItem as ComboBoxItem)?.Tag?.ToString()??"";var locations=await DeviceLocations(id);if((target.SelectedItem as ComboBoxItem)?.Tag?.ToString()==id)input.Text=locations.FirstOrDefault()?["path"]?.ToString()??"";}catch(Exception e){message.Text=e.Message;}};
        var review=new TextBlock{Text=string.Join("\n",selected.Take(8).Select(r=>r.Name))+(selected.Length>8?"\n…":""),TextWrapping=TextWrapping.Wrap,FontSize=11,Opacity=.7};body.Children.Add(review);
        var error=new TextBlock{TextWrapping=TextWrapping.Wrap,Foreground=Brush(0xe5a088),FontSize=11};body.Children.Add(error);JsonNode? plan=null;
        Button? choose=null;if(operation is "copy" or "move"){
            choose=FlatButton("Choose folder…","\uE8B7",async()=>{var popup=activePopover;await ClosePopover();try{var chosen=await PickPath("Copy destination",input.Text.Length>0?input.Text:null,deviceId:operation=="copy"?(target.SelectedItem as ComboBoxItem)?.Tag?.ToString():selected[0].Device);if(chosen!=null)input.Text=chosen;}finally{popup?.ShowAt((FrameworkElement?)batchActions??root);}});body.Children.Add(choose);
        }
        var progressText=new TextBlock{FontSize=11,TextWrapping=TextWrapping.Wrap};body.Children.Add(progressText);
        string? copyingId=null;var stop=FlatButton("Stop copy","\uE711",async()=>{if(copyingId!=null){await api.Post("workspace/cancel",new JsonObject{["id"]=copyingId});progressText.Text="Stopping after the current block or native file copy…";}});stop.Visibility=Visibility.Collapsed;body.Children.Add(stop);
        if(operation=="trash")body.Children.Add(new TextBlock{Text=remoteTrash?"Remote files move to hidden Retriever Trash beside the original file. Restore them from Activity → Review undo. Nothing is permanently erased.":"Files go to the Windows Recycle Bin.",FontSize=11,TextWrapping=TextWrapping.Wrap});
        Button? apply=null;void ApplyLabel(string text){apply!.Content=text;Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(apply,text);}
        apply=FlatButton(operation=="trash"&&!remoteTrash?"Move to Recycle Bin":"Review changes","\uE73E",async()=>{
            apply!.IsEnabled=false;try{
                if(operation=="trash"&&!remoteTrash){
                    if(api.Mode=="client"||selected.Any(r=>r.Device!=status?["local_device"]?.ToString()))throw new InvalidOperationException("Use the source device to recycle these files.");
                    var paths=selected.Select(r=>Path.GetFullPath(r.Path)).ToArray();
                    var privateState=Path.GetFullPath(api.StatePath).TrimEnd('\\','/');
                    if(paths.Any(p=>Path.GetPathRoot(p)==p||p.TrimEnd('\\','/').Equals(privateState,StringComparison.OrdinalIgnoreCase)||p.StartsWith(privateState+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)||privateState.StartsWith(p.TrimEnd('\\','/')+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)))throw new InvalidOperationException("System roots and Retriever state cannot be recycled.");
                    if(paths.Any(a=>paths.Any(b=>a!=b&&a.StartsWith(b.TrimEnd('\\','/')+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase))))throw new InvalidOperationException("Select a folder or its children, not both.");
                    foreach(var path in paths){var attributes=File.GetAttributes(path);if((attributes&FileAttributes.ReparsePoint)!=0)throw new InvalidOperationException("Linked files require review in Explorer.");}
                    foreach(var path in paths){await Task.Run(()=>{if(Directory.Exists(path))Microsoft.VisualBasic.FileIO.FileSystem.DeleteDirectory(path,Microsoft.VisualBasic.FileIO.UIOption.OnlyErrorDialogs,Microsoft.VisualBasic.FileIO.RecycleOption.SendToRecycleBin,Microsoft.VisualBasic.FileIO.UICancelOption.ThrowException);else Microsoft.VisualBasic.FileIO.FileSystem.DeleteFile(path,Microsoft.VisualBasic.FileIO.UIOption.OnlyErrorDialogs,Microsoft.VisualBasic.FileIO.RecycleOption.SendToRecycleBin,Microsoft.VisualBasic.FileIO.UICancelOption.ThrowException);});}
                    activePopover?.Hide();await RefreshCurrent();message.Text=$"Moved {paths.Length} items to the Recycle Bin";
                }else if(plan==null){
                    var items=new JsonArray();for(int i=0;i<selected.Length;i++){var row=selected[i];var item=new JsonObject{["path"]=row.Path};if(operation=="rename")item["name"]=input.Text.Replace("{name}",row.Kind=="folder"?row.Name:Path.GetFileNameWithoutExtension(row.Name)).Replace("{n}",(i+1).ToString()).Replace("{ext}",row.Kind=="folder"?"":Path.GetExtension(row.Name));else item["parent"]=input.Text;items.Add(item);}
                    plan=await api.Post("workspace/plan",new JsonObject{["device"]=selected[0].Device,["target_device"]=(target.SelectedItem as ComboBoxItem)?.Tag?.ToString()??selected[0].Device,["operation"]=operation,["scope"]="filesystem",["parent"]=input.Text,["items"]=items});input.IsReadOnly=true;target.IsEnabled=false;
                    review.Text=string.Join("\n\n",(plan["items"]?.AsArray()??[]).Select(i=>$"{i?["source"]}\n→ {i?["destination"]}"));ApplyLabel("Apply "+operation);if(choose!=null)choose.IsEnabled=false;
                }else{JsonNode receipt;if(operation=="copy"){copyingId=plan["id"]!.ToString();stop.Visibility=Visibility.Visible;receipt=await RunCopy(plan,p=>{var bytes=Number(p["completed_bytes"]);var total=Number(p["total_bytes"]);progressText.Text=$"{bytes/1048576d:N1} / {total/1048576d:N1} MiB · {p["phase"]}";});}else receipt=await api.Post("workspace/apply",new JsonObject{["id"]=plan["id"]!.DeepClone()});if(receipt["receipt"]?["ok"]?.GetValue<bool>()!=true){plan=null;input.IsReadOnly=false;target.IsEnabled=true;if(choose!=null)choose.IsEnabled=true;throw new InvalidOperationException("Operation needs inspection in Activity: "+receipt["receipt"]?["error"]);}activePopover?.Hide();await RefreshCurrent();message.Text=operation=="trash"?"Moved to Retriever Trash · restore from Activity":operation=="copy"?"Copy complete · checksums verified":receipt["state"]?.ToString()??"Check the operation receipt.";}
            }catch(Exception e){error.Text=e.Message;if(plan==null)ApplyLabel(operation=="trash"&&!remoteTrash?"Move to Recycle Bin":"Review changes");}finally{copyingId=null;stop.Visibility=Visibility.Collapsed;apply.IsEnabled=true;}
        });body.Children.Add(apply);Popover(operation=="trash"?"Recycle selected files":"Batch "+operation,new ScrollViewer{Content=body,MaxHeight=440},batchActions,390);return Task.CompletedTask;
    }
}
