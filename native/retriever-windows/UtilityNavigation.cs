using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;

namespace Retriever.Native;
public sealed partial class MainWindow {
    bool utilitiesCollapsed;
    void BuildUtilities(){
        var header=FlatButton("Utilities",utilitiesCollapsed?"\uE76C":"\uE70D",async()=>{utilitiesCollapsed=!utilitiesCollapsed;SaveFolderTree();await RefreshSidebar();});
        header.Margin=new Thickness(4,16,4,4);header.HorizontalAlignment=HorizontalAlignment.Stretch;header.HorizontalContentAlignment=HorizontalAlignment.Left;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(header,(utilitiesCollapsed?"Expand":"Collapse")+" Utilities");sidebarItems.Children.Add(header);
        if(utilitiesCollapsed)return;
        foreach(var (title,glyph,key,action) in new (string,string,string,Func<Task>)[]{("Disk Health","\uE95E","health",OpenDriveHealth),("Disk Capacity","\uEDA2","disk",OpenDiskSpace),("Disk Assistant","\uE945","assistant",OpenDiskAssistant),("Disk Burner","\uE958","burner",OpenDiskBurner)}){
            AddNav(title,glyph,key,action);var menu=new MenuFlyout();var item=new MenuFlyoutItem{Text="Open in new tab"};
            item.Click+=async(_,_)=>await Guard(async()=>{var state=EmptyTabState();state["route"]=key;state["title"]=title;state["nav"]=key;await NavigateDestination(state,true);});menu.Items.Add(item);navButtons[key].ContextFlyout=menu;
        }
    }
}
