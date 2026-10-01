# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

from dynwinrt import RoApartment, projected_lifetime_scope, to_winrt_object
from xml_generated import (
    IXmlNode,
    XmlDocument,
    XmlLoadSettings,
)
from xml_generated.windows__data__xml__dom__i_xml_document import IXmlDocument
from xml_generated.windows__data__xml__dom__i_xml_element import IXmlElement


with RoApartment(), projected_lifetime_scope() as scope:
    document = XmlDocument()
    document_view = document.as_interface(IXmlDocument)
    document_node = document.as_interface(IXmlNode)
    for view in (document, document_view):
        assert view.document_element is None
        assert view.doctype is None
        assert view.get_element_by_id("missing") is None
        assert view.implementation is not None
    for view in (document, document_node):
        assert view.owner_document is None
        assert view.parent_node is None
        assert view.previous_sibling is None
        assert view.next_sibling is None
        assert view.first_child is None
        assert view.last_child is None
    print("empty document: class and declaring-interface null results")

    document.load_xml('<root present="old"><first/><last/></root>')
    root = document.document_element
    assert root is not None and root.tag_name == "root"
    assert document_view.document_element.tag_name == "root"
    assert document.doctype is None and document_view.doctype is None
    assert document.owner_document is None and document_node.owner_document is None
    assert root.owner_document.get_xml() == document.get_xml()
    assert root.parent_node.node_name == "#document"
    first, last = root.first_child, root.last_child
    assert first is not None and last is not None
    assert first.previous_sibling is None and last.next_sibling is None
    assert first.next_sibling.node_name == "last"
    assert last.previous_sibling.node_name == "first"
    print("loaded document without DTD: present root and sibling boundaries")

    detached = document.create_element("detached")
    text = document.create_text_node("text")
    for view in (detached, text, detached.as_interface(IXmlNode), text.as_interface(IXmlNode)):
        assert view.parent_node is None
        assert view.previous_sibling is None
        assert view.next_sibling is None
        assert view.owner_document.get_xml() == document.get_xml()
        assert view.first_child is None and view.last_child is None
    root.append_child(detached.as_interface(IXmlNode))
    root.append_child(text.as_interface(IXmlNode))
    assert detached.parent_node.node_name == "root"
    assert text.parent_node.node_name == "root"
    assert detached.next_sibling.node_name == "#text"
    assert text.previous_sibling.node_name == "detached"
    assert text.next_sibling is None
    root.remove_child(detached.as_interface(IXmlNode))
    root.remove_child(text.as_interface(IXmlNode))
    for view in (detached, text):
        assert view.parent_node is None
        assert view.previous_sibling is None and view.next_sibling is None
    print("element/text: detached, attached, and removed navigation")

    element_view = root.as_interface(IXmlElement)
    for view in (root, element_view):
        assert view.get_attribute_node("missing") is None
        assert view.get_attribute_node("present").value == "old"
    attribute = document.create_attribute("new")
    attribute.value = "first"
    assert root.set_attribute_node(attribute) is None
    assert element_view.get_attribute_node("new").value == "first"
    replacement = document.create_attribute("new")
    replacement.value = "second"
    previous = element_view.set_attribute_node(replacement)
    assert previous is not None and previous.value == "first"
    assert previous._obj.identity_raw() == attribute._obj.identity_raw()
    assert root.get_attribute_node("new").value == "second"
    print("attributes: missing/present lookup and added/replaced previous object")

    namespace = scope.track(to_winrt_object("urn:xml-test"))
    for view in (root, element_view):
        assert view.get_attribute_node_ns(namespace, "missing") is None
    namespaced = document.create_attribute_ns(namespace, "test:flag")
    namespaced.value = "first"
    assert element_view.set_attribute_node_ns(namespaced) is None
    assert root.get_attribute_node_ns(namespace, "flag").value == "first"
    replacement_ns = document.create_attribute_ns(namespace, "test:flag")
    replacement_ns.value = "second"
    previous_ns = root.set_attribute_node_ns(replacement_ns)
    assert previous_ns is not None and previous_ns.value == "first"
    assert previous_ns._obj.identity_raw() == namespaced._obj.identity_raw()
    assert element_view.get_attribute_node_ns(namespace, "flag").value == "second"
    print("namespace attributes: explicitly boxed Object and both replacement states")

    settings = XmlLoadSettings()
    settings.prohibit_dtd = False
    settings.resolve_externals = False
    with_dtd = XmlDocument()
    with_dtd.load_xml(
        '<!DOCTYPE root [<!ELEMENT root ANY><!ATTLIST root id ID #IMPLIED>]>'
        '<root id="known"/>',
        settings,
    )
    dtd_view = with_dtd.as_interface(IXmlDocument)
    for view in (with_dtd, dtd_view):
        assert view.doctype is not None and view.doctype.name == "root"
        assert view.document_element.tag_name == "root"
        assert view.get_element_by_id("missing") is None
        assert view.get_element_by_id("known").tag_name == "root"
    assert with_dtd.owner_document is None
    assert with_dtd.as_interface(IXmlNode).owner_document is None
    print("internal DTD: present doctype and missing/present ID lookup; no external resources")

print("native XML nullable-result states passed; references released before apartment exit")
