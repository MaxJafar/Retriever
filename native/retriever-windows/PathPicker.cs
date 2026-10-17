using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using System.Text.Json.Nodes;

namespace Retriever.Native;

public sealed partial class MainWindow {
    sealed record PathChoice(string Name,string Path,bool Folder) {
        public string Glyph=>Folder?"\uE8B7":"\uE8A5";
    }

    // Uses the same live, permission-checked filesystem API as the main browser.
    // Picking a path neither starts crawling nor changes the current explorer tab.
    async Task<string?> PickPath(string title,string? initial=null,string[]? extensions=null,string? deviceId=null){
        var device=deviceId??config?["devices"]?.AsArray().FirstOrDefault(d=>d?["transport"]?.ToString()=="local"&&d?["enabled"]?.GetValue<bool>()!=false)?["id"]?.ToString()
            ??throw new InvalidOperationException("Connect this computer before choosing a local path.");
        var body=new StackPanel{Spacing=10,Width=Math.Max(280,Math.Min(600,root.ActualWidth-100))};
        var address=new TextBox{PlaceholderText="Folder path",FontSize=12};
        AutomationProperties.SetName(address,"Folder path");
        var places=new ComboBox{PlaceholderText="Home and drives",DisplayMemberPath="Name",HorizontalAlignment=HorizontalAlignment.Stretch};
        var navigation=new StackPanel{Orientation=Orientation.Horizontal,Spacing=6};
        var listing=new ListView{Height=Math.Max(160,Math.Min(310,root.ActualHeight-310)),SelectionMode=ListViewSelectionMode.Single,IsItemClickEnabled=false};
        listing.ItemTemplate=(DataTemplate)XamlReader.Load("<DataTemplate xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation'><Grid Height='32' ColumnSpacing='10'><Grid.ColumnDefinitions><ColumnDefinition Width='22'/><ColumnDefinition Width='*'/></Grid.ColumnDefinitions><FontIcon Glyph='{Binding Glyph}' FontSize='16' Foreground='#E5BD4C'/><TextBlock Grid.Column='1' Text='{Binding Name}' VerticalAlignment='Center' TextTrimming='CharacterEllipsis'/></Grid></DataTemplate>");
        var feedback=new TextBlock{FontSize=11,Opacity=.7,TextWrapping=TextWrapping.Wrap};
        var progress=new ProgressBar{IsIndeterminate=true,Height=2,Visibility=Visibility.Collapsed};
        body.Children.Add(places);body.Children.Add(navigation);body.Children.Add(address);body.Children.Add(progress);body.Children.Add(listing);body.Children.Add(feedback);
        var dialog=new ContentDialog{Title=title,Content=body,XamlRoot=root.XamlRoot,RequestedTheme=root.RequestedTheme,PrimaryButtonText=extensions==null?"Choose folder":"Choose file",CloseButtonText="Cancel",DefaultButton=ContentDialogButton.None,IsPrimaryButtonEnabled=false};
        string current="",parent="";int offset=0;int? next=null;bool busy=false,closed=false;long generation=0;
        var history=new Stack<string>();Button? back=null,up=null,more=null;
        void Selection(){dialog.IsPrimaryButtonEnabled=!busy&&string.Equals(address.Text.Trim(),current,StringComparison.OrdinalIgnoreCase)&&(extensions==null?current.Length>0:listing.SelectedItem is PathChoice{Folder:false});}
        async Task Load(string path,bool remember=true,bool append=false){
            var request=++generation;busy=true;Selection();progress.Visibility=Visibility.Visible;feedback.Text="Loading folder…";
            try{
                var reply=await api.Post("explorer/browse",new JsonObject{["device"]=device,["path"]=path,["offset"]=append?next??0:0,["sort"]="name"});
                if(closed||request!=generation)return;
                if(!append){if(remember&&current.Length>0&&current!=path)history.Push(current);listing.Items.Clear();offset=0;}
                current=reply["path"]?.ToString()??path;parent=reply["parent"]?.ToString()??"";address.Text=current;
                foreach(var item in reply["entries"]?.AsArray()??[]){if(item==null)continue;var folder=item["kind"]?.ToString()=="folder";var name=item["name"]?.ToString()??"";
                    if(folder||extensions!=null&&extensions.Contains(System.IO.Path.GetExtension(name),StringComparer.OrdinalIgnoreCase))listing.Items.Add(new PathChoice(name,item["path"]!.ToString(),folder));}
                next=reply["next_offset"]?.GetValue<int>();offset+=reply["entries"]?.AsArray().Count??0;
                feedback.Text=$"{listing.Items.Count:N0} {(extensions==null?"folders":"items")} shown"+(next!=null?" · More items available":"")+(Number(reply["skipped"])>0?$" · {Number(reply["skipped"])} inaccessible items":"");
                up!.IsEnabled=parent.Length>0;back!.IsEnabled=history.Count>0;more!.IsEnabled=next!=null;
            }catch(Exception ex){if(!closed&&request==generation)feedback.Text=ex.Message;}
            finally{if(request==generation){busy=false;progress.Visibility=Visibility.Collapsed;Selection();}}
        }
        back=FlatButton("Back","\uE76B",async()=>{if(!busy&&history.TryPop(out var previous))await Load(previous,false);},true);
        up=FlatButton("Parent folder","\uE70E",async()=>{if(!busy&&parent.Length>0)await Load(parent);},true);
        more=FlatButton("More items","\uE712",async()=>{if(!busy&&next!=null)await Load(current,false,true);});
        navigation.Children.Add(back);navigation.Children.Add(up);navigation.Children.Add(more);
        listing.SelectionChanged+=(_,_)=>Selection();
        address.TextChanged+=(_,_)=>Selection();
        listing.DoubleTapped+=async(_,_)=>{if(!busy&&listing.SelectedItem is PathChoice{Folder:true} folder)await Load(folder.Path);};
        listing.KeyDown+=async(_,e)=>{if(e.Key==Windows.System.VirtualKey.Enter&&!busy&&listing.SelectedItem is PathChoice{Folder:true} folder){e.Handled=true;await Load(folder.Path);}};
        address.KeyDown+=async(_,e)=>{if(e.Key==Windows.System.VirtualKey.Enter&&!busy){e.Handled=true;await Load(address.Text.Trim());}};
        places.SelectionChanged+=async(_,_)=>{if(places.SelectedItem is PathChoice place&&!busy)await Load(place.Path);};
        dialog.Opened+=async(_,_)=>{
            try{var data=await api.Post("explorer/locations",new JsonObject{["device"]=device});if(closed)return;
                foreach(var item in data["locations"]?.AsArray()??[])if(item!=null)places.Items.Add(new PathChoice(item["name"]!.ToString(),item["path"]!.ToString(),true));
                var start=initial;if(string.IsNullOrWhiteSpace(start))start=(places.Items.FirstOrDefault() as PathChoice)?.Path;
                if(!string.IsNullOrWhiteSpace(start))await Load(start,false);
            }catch(Exception ex){feedback.Text=ex.Message;}
        };
        dialog.Closed+=(_,_)=>{closed=true;generation++;};
        var previousDialog=dialogVisible;dialogVisible=true;
        try{var result=await dialog.ShowAsync();return result==ContentDialogResult.Primary?(listing.SelectedItem as PathChoice)?.Path??current:null;}
        finally{dialogVisible=previousDialog;}
    }
}
