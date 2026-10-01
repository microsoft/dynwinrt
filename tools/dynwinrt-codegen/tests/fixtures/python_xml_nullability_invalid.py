# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.
# pyright: strict

from dynwinrt import to_winrt_object
from xml_generated import (
    IXmlNode,
    XmlDocument,
    XmlElement,
    XmlText,
)
from xml_generated.windows__data__xml__dom__i_xml_document import IXmlDocument
from xml_generated.windows__data__xml__dom__i_xml_element import IXmlElement


def unsafe_reads(
    document: XmlDocument,
    element: XmlElement,
    text: XmlText,
    node: IXmlNode,
    document_view: IXmlDocument,
    element_view: IXmlElement,
) -> None:
    print(document.document_element.tag_name)  # unsafe
    print(document.doctype.name)  # unsafe
    print(document.owner_document.node_name)  # unsafe
    print(element.parent_node.node_name)  # unsafe
    print(element.next_sibling.node_name)  # unsafe
    print(text.previous_sibling.node_name)  # unsafe
    print(node.parent_node.node_name)  # unsafe
    print(node.previous_sibling.node_name)  # unsafe
    print(node.next_sibling.node_name)  # unsafe
    print(node.owner_document.node_name)  # unsafe
    print(element.get_attribute_node("missing").name)  # unsafe
    print(element.set_attribute_node(document.create_attribute("a")).name)  # unsafe
    print(element.get_attribute_node_ns(to_winrt_object("urn:xml-test"), "missing").name)  # unsafe
    print(document_view.document_element.tag_name)  # unsafe
    print(document_view.doctype.name)  # unsafe
    print(document_view.get_element_by_id("missing").tag_name)  # unsafe
    print(element_view.get_attribute_node("missing").name)  # unsafe
    print(element_view.set_attribute_node(document.create_attribute("a")).name)  # unsafe
    print(element_view.get_attribute_node_ns(to_winrt_object("urn:xml-test"), "missing").name)  # unsafe
    print(element_view.set_attribute_node_ns(document.create_attribute("a")).name)  # unsafe
