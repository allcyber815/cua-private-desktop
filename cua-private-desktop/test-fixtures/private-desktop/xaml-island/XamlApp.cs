using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.XamlTypeInfo;
using System.Collections.Generic;

namespace WebGPT.XamlIslandFixture;

internal sealed class XamlApp : Microsoft.UI.Xaml.Application, IXamlMetadataProvider
{
    private readonly WindowsXamlManager _manager;
    private readonly List<IXamlMetadataProvider> _providers = new();

    public XamlApp()
    {
        _providers.Add(new XamlControlsXamlMetaDataProvider());
        _manager = WindowsXamlManager.InitializeForCurrentThread();
    }

    protected override void OnLaunched(Microsoft.UI.Xaml.LaunchActivatedEventArgs args)
    {
        Resources.MergedDictionaries.Add(new Microsoft.UI.Xaml.Controls.XamlControlsResources());
    }

    IXamlType? IXamlMetadataProvider.GetXamlType(string fullName)
    {
        foreach (var provider in _providers)
        {
            var value = provider.GetXamlType(fullName);
            if (value != null) return value;
        }
        return null;
    }

    IXamlType? IXamlMetadataProvider.GetXamlType(System.Type type)
    {
        foreach (var provider in _providers)
        {
            var value = provider.GetXamlType(type);
            if (value != null) return value;
        }
        return null;
    }

    XmlnsDefinition[] IXamlMetadataProvider.GetXmlnsDefinitions()
    {
        var values = new List<XmlnsDefinition>();
        foreach (var provider in _providers) values.AddRange(provider.GetXmlnsDefinitions());
        return values.ToArray();
    }
}
