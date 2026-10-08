# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.
# pyright: strict

from typing import assert_type

from dynwinrt import to_winrt_object
from xml_generated import (
    IXmlNode,
    Uri,
    XmlAttribute,
    XmlDocumentType,
    XmlDomImplementation,
    XmlElement,
    XmlNodeList,
    XmlText,
    WwwFormUrlDecoder,
)
from xml_generated.windows.data.xml.dom import XmlDocument
from xml_generated.windows__data__xml__dom__i_xml_document import IXmlDocument
from xml_generated.windows__data__xml__dom__i_xml_element import IXmlElement


def guarded_reads(
    document: XmlDocument,
    element: XmlElement,
    text: XmlText,
    node: IXmlNode,
    document_view: IXmlDocument,
    element_view: IXmlElement,
) -> None:
    root = document.document_element
    assert_type(root, XmlElement | None)
    if root is not None:
        print(root.tag_name)

    doctype = document.doctype
    assert_type(doctype, XmlDocumentType | None)
    if doctype is not None:
        print(doctype.name)

    for owner in (document.owner_document, node.owner_document):
        assert_type(owner, XmlDocument | None)
        if owner is not None:
            print(owner.node_name)

    for relative in (
        element.parent_node,
        element.previous_sibling,
        element.next_sibling,
        text.parent_node,
        text.previous_sibling,
        text.next_sibling,
        node.parent_node,
        node.previous_sibling,
        node.next_sibling,
        node.first_child,
        node.last_child,
    ):
        assert_type(relative, IXmlNode | None)
        if relative is not None:
            print(relative.node_name)

    namespace = to_winrt_object("urn:xml-test")
    for attribute in (
        element.get_attribute_node("missing"),
        element.get_attribute_node_ns(namespace, "missing"),
        element.set_attribute_node(document.create_attribute("a")),
        element.set_attribute_node_ns(document.create_attribute("b")),
        element_view.get_attribute_node("missing"),
        element_view.get_attribute_node_ns(namespace, "missing"),
        element_view.set_attribute_node(document.create_attribute("c")),
        element_view.set_attribute_node_ns(document.create_attribute("d")),
    ):
        assert_type(attribute, XmlAttribute | None)
        if attribute is not None:
            print(attribute.name)

    interface_root = document_view.document_element
    assert_type(interface_root, XmlElement | None)
    if interface_root is not None:
        print(interface_root.tag_name)
    interface_doctype = document_view.doctype
    assert_type(interface_doctype, XmlDocumentType | None)
    if interface_doctype is not None:
        print(interface_doctype.name)
    identified = document_view.get_element_by_id("missing")
    assert_type(identified, XmlElement | None)
    if identified is not None:
        print(identified.tag_name)

    # Ordinary factories, collection containers and successful removals stay non-null.
    assert_type(document.create_element("root"), XmlElement)
    assert_type(document.create_text_node("text"), XmlText)
    assert_type(document.create_attribute("a"), XmlAttribute)
    assert_type(document.implementation, XmlDomImplementation)
    assert_type(document_view.implementation, XmlDomImplementation)
    assert_type(document_view.create_element("root"), XmlElement)
    assert_type(document.child_nodes, XmlNodeList)
    assert_type(element.select_nodes("*"), XmlNodeList)
    assert_type(element.get_attribute("missing"), str)
    assert_type(element.remove_attribute_node(document.create_attribute("a")), XmlAttribute)
    assert_type(node.clone_node(False), IXmlNode)
    assert_type(Uri("https://example.com").query_parsed, WwwFormUrlDecoder)
