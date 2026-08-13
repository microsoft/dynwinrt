# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""
End-to-end tests for the Python bindings using real WinRT APIs.

These tests exercise the full dynamic invocation pipeline:
  Python → PyO3 → dynwinrt core → libffi → COM vtable → WinRT API

Each test registers interface signatures, calls real WinRT methods, and
verifies the results against known expected values.

Requires: Windows 10/11 with standard SDK (no extra installs needed).
"""

from dynwinrt import (
    DynWinRTType,
    DynWinRTMethodSig,
    DynWinRTValue,
    DynWinRTArray,
    DynWinRTStruct,
    WinGUID,
    ro_initialize,
)

# Initialize WinRT once for the entire module
ro_initialize(1)


# ======================================================================
# Helper: register an interface and add named methods in one go
# ======================================================================

def _register(name, iid_str, methods):
    """
    Register an interface and add methods.
    
    methods: list of (name, [in_types], [out_types])
    Returns (interface_type, {name: method_handle})
    """
    iid = WinGUID.parse(iid_str)
    itype = DynWinRTType.register_interface(name, iid)
    for mname, ins, outs in methods:
        sig = DynWinRTMethodSig()
        for t in ins:
            sig = sig.add_in(t)
        for t in outs:
            sig = sig.add_out(t)
        itype = itype.add_method(mname, sig)
    handles = {}
    for mname, _, _ in methods:
        handles[mname] = itype.method_by_name(mname)
    return itype, iid, handles


# ======================================================================
# 1. XmlDocument — void methods, object returns, string properties
# ======================================================================

class TestXmlDocument:
    """
    Tests: XmlDocument.new() → LoadXml(hstring) → DocumentElement() → IXmlNode
    Exercises: activation (parameterless ctor), void method, object return,
    string getters via different interfaces on the same object (QI/cast).
    """

    # IIDs
    IXMLDOCUMENT_IID = "f7f3a506-1e87-42d6-bcfb-b8c809fa5494"
    IXMLDOCUMENTIO_IID = "6cd0e74e-ee65-4489-9ebf-ca43e87ba637"
    IXMLNODE_IID = "1c741d59-2122-47d5-a856-83f3d4214875"
    IXMLNODESERIALIZER_IID = "5cc5b382-e6dd-4991-abef-06d8d2e7bd0c"
    IXMLELEMENT_IID = "2dfb8a1f-6b10-4ef8-9f83-efcce8faec37"

    def _setup_interfaces(self):
        hstring = DynWinRTType.hstring()
        obj = DynWinRTType.object()

        # IXmlDocument: [6] Doctype, [7] Implementation, [8] DocumentElement, ...
        _, doc_iid, doc_h = _register("E2E_IXmlDocument", self.IXMLDOCUMENT_IID, [
            ("Doctype", [], [obj]),           # [6]
            ("Implementation", [], [obj]),    # [7]
            ("DocumentElement", [], [obj]),   # [8]
        ])

        # IXmlDocumentIO: [6] LoadXml(hstring) → void
        _, docio_iid, docio_h = _register("E2E_IXmlDocumentIO", self.IXMLDOCUMENTIO_IID, [
            ("LoadXml", [hstring], []),       # [6]
        ])

        # IXmlNode: [6..9] NodeValue, SetNodeValue, NodeType, NodeName
        _, node_iid, node_h = _register("E2E_IXmlNode", self.IXMLNODE_IID, [
            ("NodeValue", [], [obj]),         # [6]
            ("SetNodeValue", [obj], []),      # [7]
            ("NodeType", [], [DynWinRTType.i32_type()]),  # [8] enum as i32
            ("NodeName", [], [hstring]),      # [9]
        ])

        # IXmlNodeSerializer: [6] GetXml, [7] InnerText
        _, serializer_iid, serializer_h = _register("E2E_IXmlNodeSerializer", self.IXMLNODESERIALIZER_IID, [
            ("GetXml", [], [hstring]),        # [6]
            ("InnerText", [], [hstring]),     # [7]
        ])

        # IXmlElement: [6] TagName, [7] GetAttribute(hstring) → hstring
        _, elem_iid, elem_h = _register("E2E_IXmlElement", self.IXMLELEMENT_IID, [
            ("TagName", [], [hstring]),                     # [6]
            ("GetAttribute", [hstring], [hstring]),         # [7]
        ])

        return {
            "doc_iid": doc_iid, "doc_h": doc_h,
            "docio_iid": docio_iid, "docio_h": docio_h,
            "node_iid": node_iid, "node_h": node_h,
            "serializer_iid": serializer_iid, "serializer_h": serializer_h,
            "elem_iid": elem_iid, "elem_h": elem_h,
        }

    def test_load_xml_and_read_properties(self):
        """Create XmlDocument, LoadXml, read DocumentElement properties."""
        ctx = self._setup_interfaces()

        # Activate XmlDocument (parameterless constructor → IActivationFactory.ActivateInstance)
        factory = DynWinRTValue.activation_factory("Windows.Data.Xml.Dom.XmlDocument")
        doc = factory.activate()

        # Cast to IXmlDocumentIO and call LoadXml (void method)
        doc_io = doc.cast(ctx["docio_iid"])
        ctx["docio_h"]["LoadXml"].invoke(doc_io, [DynWinRTValue.from_hstring('<root attr="val">hello world</root>')])

        # Cast to IXmlDocument and call DocumentElement → object
        doc_cast = doc.cast(ctx["doc_iid"])
        elem_obj = ctx["doc_h"]["DocumentElement"].invoke(doc_cast, [])

        # Cast element to IXmlNode and read NodeName
        elem_node = elem_obj.cast(ctx["node_iid"])
        assert ctx["node_h"]["NodeName"].get_string(elem_node) == "root"

        # Cast element to IXmlNodeSerializer and read InnerText and GetXml
        elem_ser = elem_obj.cast(ctx["serializer_iid"])
        assert ctx["serializer_h"]["InnerText"].get_string(elem_ser) == "hello world"
        xml = ctx["serializer_h"]["GetXml"].get_string(elem_ser)
        assert "hello world" in xml
        assert "root" in xml

    def test_element_tag_name_and_attribute(self):
        """Read TagName and GetAttribute via IXmlElement."""
        ctx = self._setup_interfaces()

        factory = DynWinRTValue.activation_factory("Windows.Data.Xml.Dom.XmlDocument")
        doc = factory.activate()

        doc_io = doc.cast(ctx["docio_iid"])
        ctx["docio_h"]["LoadXml"].invoke(doc_io, [DynWinRTValue.from_hstring('<item id="42">content</item>')])

        doc_cast = doc.cast(ctx["doc_iid"])
        elem_obj = ctx["doc_h"]["DocumentElement"].invoke(doc_cast, [])

        # IXmlElement.TagName
        elem_cast = elem_obj.cast(ctx["elem_iid"])
        assert ctx["elem_h"]["TagName"].get_string(elem_cast) == "item"

        # IXmlElement.GetAttribute("id")
        attr_result = ctx["elem_h"]["GetAttribute"].invoke(elem_cast, [DynWinRTValue.from_hstring("id")])
        assert attr_result.to_string() == "42"


# ======================================================================
# 2. Geopoint — struct input/output (3×f64 BasicGeoposition)
# ======================================================================

class TestGeopoint:
    """
    Tests: Create Geopoint from struct → read Position struct back.
    Exercises: struct as in-parameter, struct as out-parameter, f64 fields.
    """

    IGEOPOINTFACTORY_IID = "db6b8d33-76bd-4e30-8af7-a844dc37b7a0"
    IGEOPOINT_IID = "6bfa00eb-e56e-49bb-9caf-cbaa78a8bcef"

    def _geo_struct_type(self):
        """BasicGeoposition: { Latitude: f64, Longitude: f64, Altitude: f64 }"""
        f64 = DynWinRTType.f64_type()
        return DynWinRTType.struct_type("E2E_BasicGeoposition", [f64, f64, f64])

    def test_create_geopoint_and_read_position(self):
        """Round-trip: create Geopoint from struct, read struct back."""
        geo_type = self._geo_struct_type()
        obj_type = DynWinRTType.object()

        # Register IGeopointFactory: [6] Create(BasicGeoposition) → Geopoint
        _, factory_iid, factory_h = _register("E2E_IGeopointFactory", self.IGEOPOINTFACTORY_IID, [
            ("Create", [geo_type], [obj_type]),
        ])

        # Register IGeopoint: [6] Position → BasicGeoposition
        _, gp_iid, gp_h = _register("E2E_IGeopoint", self.IGEOPOINT_IID, [
            ("get_Position", [], [geo_type]),
        ])

        # Create struct with known values
        geo_struct = DynWinRTStruct.create(geo_type)
        geo_struct.set_f64(0, 47.643)     # Latitude
        geo_struct.set_f64(1, -122.131)   # Longitude
        geo_struct.set_f64(2, 100.0)      # Altitude

        # Get factory and create Geopoint
        factory = DynWinRTValue.activation_factory("Windows.Devices.Geolocation.Geopoint")
        factory_cast = factory.cast(factory_iid)
        geopoint = factory_h["Create"].invoke(factory_cast, [geo_struct.to_value()])

        # Cast to IGeopoint and read Position
        gp_cast = geopoint.cast(gp_iid)
        position_val = gp_h["get_Position"].invoke(gp_cast, [])

        # The result is a struct value — extract it
        assert position_val.is_struct()
        pos = position_val.as_struct()
        assert abs(pos.get_f64(0) - 47.643) < 1e-6     # Latitude
        assert abs(pos.get_f64(1) - (-122.131)) < 1e-6  # Longitude
        assert abs(pos.get_f64(2) - 100.0) < 1e-6       # Altitude

    def test_geopoint_different_coordinates(self):
        """Verify with different coordinates (0,0,0 — null island)."""
        geo_type = self._geo_struct_type()

        _, factory_iid, factory_h = _register("E2E_IGeopointFactory2", self.IGEOPOINTFACTORY_IID, [
            ("Create", [geo_type], [DynWinRTType.object()]),
        ])
        _, gp_iid, gp_h = _register("E2E_IGeopoint2", self.IGEOPOINT_IID, [
            ("get_Position", [], [geo_type]),
        ])

        geo_struct = DynWinRTStruct.create(geo_type)
        geo_struct.set_f64(0, 0.0)
        geo_struct.set_f64(1, 0.0)
        geo_struct.set_f64(2, 0.0)

        factory = DynWinRTValue.activation_factory("Windows.Devices.Geolocation.Geopoint")
        geopoint = factory_h["Create"].invoke(factory.cast(factory_iid), [geo_struct.to_value()])

        pos = gp_h["get_Position"].invoke(geopoint.cast(gp_iid), [])
        s = pos.as_struct()
        assert abs(s.get_f64(0)) < 1e-6
        assert abs(s.get_f64(1)) < 1e-6
        assert abs(s.get_f64(2)) < 1e-6


# ======================================================================
# 3. PropertyValue — scalar boxing/unboxing + array pass/receive
# ======================================================================

class TestPropertyValue:
    """
    Tests: Create PropertyValue from scalars and arrays, read back values.
    Exercises: i32, f64, bool, hstring boxing, array pass-in, array receive.
    """

    IPVSTATICS_IID = "629bdbc8-d932-4ff4-96b9-8d96c5c1e858"
    IPV_IID = "4bd682dd-7554-40e9-9a9b-82654ede7e62"

    def _setup(self):
        obj = DynWinRTType.object()
        i32 = DynWinRTType.i32_type()
        f64 = DynWinRTType.f64_type()
        bool_t = DynWinRTType.bool_type()
        hstr = DynWinRTType.hstring()

        # IPropertyValueStatics: vtable[6..39]
        # We only register the methods we need with placeholder skips
        statics_methods = [
            ("CreateEmpty", [], [obj]),           # [6]
            ("CreateUInt8", [DynWinRTType.u8_type()], [obj]),  # [7]
            ("CreateInt16", [DynWinRTType.i16_type()], [obj]), # [8]
            ("CreateUInt16", [DynWinRTType.u16_type()], [obj]),# [9]
            ("CreateInt32", [i32], [obj]),         # [10]
            ("CreateUInt32", [DynWinRTType.u32_type()], [obj]),# [11]
            ("CreateInt64", [DynWinRTType.i64_type()], [obj]), # [12]
            ("CreateUInt64", [DynWinRTType.u64_type()], [obj]),# [13]
            ("CreateSingle", [DynWinRTType.f32_type()], [obj]),# [14]
            ("CreateDouble", [f64], [obj]),        # [15]
            ("CreateChar16", [DynWinRTType.char16()], [obj]),  # [16]
            ("CreateBoolean", [bool_t], [obj]),    # [17]
            ("CreateString", [hstr], [obj]),       # [18]
        ]
        _, statics_iid, statics_h = _register("E2E_IPropertyValueStatics", self.IPVSTATICS_IID, statics_methods)

        # IPropertyValue getter methods (vtable[6..])
        pv_methods = [
            ("Type", [], [i32]),                  # [6] PropertyType enum
            ("IsNumericScalar", [], [bool_t]),     # [7]
            ("GetUInt8", [], [DynWinRTType.u8_type()]),  # [8]
            ("GetInt16", [], [DynWinRTType.i16_type()]), # [9]
            ("GetUInt16", [], [DynWinRTType.u16_type()]),# [10]
            ("GetInt32", [], [i32]),               # [11]
            ("GetUInt32", [], [DynWinRTType.u32_type()]),# [12]
            ("GetInt64", [], [DynWinRTType.i64_type()]), # [13]
            ("GetUInt64", [], [DynWinRTType.u64_type()]),# [14]
            ("GetSingle", [], [DynWinRTType.f32_type()]),# [15]
            ("GetDouble", [], [f64]),              # [16]
            ("GetChar16", [], [DynWinRTType.char16()]),  # [17]
            ("GetBoolean", [], [bool_t]),          # [18]
            ("GetString", [], [hstr]),             # [19]
        ]
        _, pv_iid, pv_h = _register("E2E_IPropertyValue", self.IPV_IID, pv_methods)

        factory = DynWinRTValue.activation_factory("Windows.Foundation.PropertyValue")
        factory_cast = factory.cast(statics_iid)
        return statics_h, pv_iid, pv_h, factory_cast

    def test_create_and_get_int32(self):
        """Box an i32 and unbox it back."""
        statics_h, pv_iid, pv_h, factory = self._setup()
        pv = statics_h["CreateInt32"].invoke(factory, [DynWinRTValue.from_i32(42)])
        pv_cast = pv.cast(pv_iid)
        assert pv_h["GetInt32"].get_i32(pv_cast) == 42

    def test_create_and_get_double(self):
        """Box an f64 and unbox it back."""
        statics_h, pv_iid, pv_h, factory = self._setup()
        pv = statics_h["CreateDouble"].invoke(factory, [DynWinRTValue.from_f64(3.14159)])
        pv_cast = pv.cast(pv_iid)
        result = pv_h["GetDouble"].invoke(pv_cast, [])
        assert abs(result.to_float() - 3.14159) < 1e-10

    def test_create_and_get_boolean(self):
        """Box a bool and unbox it back."""
        statics_h, pv_iid, pv_h, factory = self._setup()
        pv = statics_h["CreateBoolean"].invoke(factory, [DynWinRTValue.from_bool(True)])
        pv_cast = pv.cast(pv_iid)
        assert pv_h["GetBoolean"].get_bool(pv_cast) is True

    def test_create_and_get_string(self):
        """Box a string and unbox it back."""
        statics_h, pv_iid, pv_h, factory = self._setup()
        pv = statics_h["CreateString"].invoke(factory, [DynWinRTValue.from_hstring("hello dynwinrt")])
        pv_cast = pv.cast(pv_iid)
        assert pv_h["GetString"].get_string(pv_cast) == "hello dynwinrt"

    def test_is_numeric_scalar(self):
        """IsNumericScalar returns False for both int and string PropertyValues
        (this matches the actual Windows API behavior)."""
        statics_h, pv_iid, pv_h, factory = self._setup()
        
        pv_int = statics_h["CreateInt32"].invoke(factory, [DynWinRTValue.from_i32(1)])
        assert pv_h["IsNumericScalar"].get_bool(pv_int.cast(pv_iid)) is False
        
        pv_str = statics_h["CreateString"].invoke(factory, [DynWinRTValue.from_hstring("x")])
        assert pv_h["IsNumericScalar"].get_bool(pv_str.cast(pv_iid)) is False


# ======================================================================
# 4. Buffer — u32 getters/setters, simple object lifecycle
# ======================================================================

class TestBuffer:
    """
    Tests: Create Buffer(capacity) → read Capacity, Length → set Length.
    Exercises: u32 return, void setter, property round-trip.
    """

    IBUFFERFACTORY_IID = "71af914d-c10f-484b-bc50-14bc623b3a27"
    IBUFFER_IID = "905a0fe0-bc53-11df-8c49-001e4fc686da"

    def _setup(self):
        u32 = DynWinRTType.u32_type()
        obj = DynWinRTType.object()

        _, factory_iid, factory_h = _register("E2E_IBufferFactory", self.IBUFFERFACTORY_IID, [
            ("Create", [u32], [obj]),             # [6]
        ])

        _, buf_iid, buf_h = _register("E2E_IBuffer", self.IBUFFER_IID, [
            ("get_Capacity", [], [u32]),           # [6]
            ("get_Length", [], [u32]),              # [7]
            ("put_Length", [u32], []),              # [8]
        ])

        factory = DynWinRTValue.activation_factory("Windows.Storage.Streams.Buffer")
        factory_cast = factory.cast(factory_iid)
        return factory_h, buf_iid, buf_h, factory_cast

    def test_create_buffer_and_read_capacity(self):
        """Create buffer with capacity 1024, verify Capacity=1024."""
        factory_h, buf_iid, buf_h, factory = self._setup()
        buf = factory_h["Create"].invoke(factory, [DynWinRTValue.from_u32(1024)])
        buf_cast = buf.cast(buf_iid)
        
        cap = buf_h["get_Capacity"].invoke(buf_cast, [])
        assert cap.to_int() == 1024

    def test_buffer_length_default_zero(self):
        """New buffer should have Length=0."""
        factory_h, buf_iid, buf_h, factory = self._setup()
        buf = factory_h["Create"].invoke(factory, [DynWinRTValue.from_u32(512)])
        buf_cast = buf.cast(buf_iid)
        
        length = buf_h["get_Length"].invoke(buf_cast, [])
        assert length.to_int() == 0

    def test_buffer_set_length(self):
        """Set length and read back."""
        factory_h, buf_iid, buf_h, factory = self._setup()
        buf = factory_h["Create"].invoke(factory, [DynWinRTValue.from_u32(1024)])
        buf_cast = buf.cast(buf_iid)

        # Set length to 512
        buf_h["put_Length"].invoke(buf_cast, [DynWinRTValue.from_u32(512)])

        # Read back
        length = buf_h["get_Length"].invoke(buf_cast, [])
        assert length.to_int() == 512

    def test_buffer_multiple_sizes(self):
        """Create buffers with different sizes."""
        factory_h, buf_iid, buf_h, factory = self._setup()
        
        for size in [0, 1, 100, 4096, 65536]:
            buf = factory_h["Create"].invoke(factory, [DynWinRTValue.from_u32(size)])
            buf_cast = buf.cast(buf_iid)
            cap = buf_h["get_Capacity"].invoke(buf_cast, [])
            assert cap.to_int() == size, f"Expected capacity {size}, got {cap.to_int()}"


# ======================================================================
# 5. Uri (extended) — more thorough string/int/bool coverage
# ======================================================================

class TestUriExtended:
    """
    Extended Uri tests beyond what test_basic.py covers.
    Exercises: multiple string getters, port (i32), suspicious (bool),
    equals method (2 inputs), and factory method with result.
    """

    FACTORY_IID = "44a9796f-723e-4fdf-a218-033e75b0c084"
    URI_IID = "9e365e57-48b2-4160-956f-c7385120bbfc"

    def _make_uri(self, url):
        hstr = DynWinRTType.hstring()
        obj = DynWinRTType.object()

        _, factory_iid, factory_h = _register("E2E_IUriFactory", self.FACTORY_IID, [
            ("CreateUri", [hstr], [obj]),
        ])

        _, uri_iid, uri_h = _register("E2E_IUriClass", self.URI_IID, [
            ("get_AbsoluteUri", [], [hstr]),       # [6]
            ("get_DisplayUri", [], [hstr]),         # [7]
            ("get_Domain", [], [hstr]),             # [8]
            ("get_Extension", [], [hstr]),          # [9]
            ("get_Fragment", [], [hstr]),            # [10]
            ("get_Host", [], [hstr]),               # [11]
            ("get_Password", [], [hstr]),            # [12]
            ("get_Path", [], [hstr]),                # [13]
            ("get_Query", [], [hstr]),               # [14]
            ("get_QueryParsed", [], [obj]),          # [15]
            ("get_RawUri", [], [hstr]),              # [16]
            ("get_SchemeName", [], [hstr]),          # [17]
            ("get_UserName", [], [hstr]),            # [18]
            ("get_Port", [], [DynWinRTType.i32_type()]),  # [19]
            ("get_Suspicious", [], [DynWinRTType.bool_type()]),  # [20]
            ("Equals", [obj], [DynWinRTType.bool_type()]),  # [21]
        ])

        factory = DynWinRTValue.activation_factory("Windows.Foundation.Uri")
        uri = factory_h["CreateUri"].invoke(factory.cast(factory_iid), [DynWinRTValue.from_hstring(url)])
        return uri_iid, uri_h, uri, factory_iid, factory_h, factory

    def test_complex_url_parsing(self):
        """Parse a complex URL with all components."""
        uri_iid, h, uri, *_ = self._make_uri("https://user:pass@example.com:8080/path/to/resource?key=value&a=b#section")
        u = uri.cast(uri_iid)

        assert h["get_SchemeName"].get_string(u) == "https"
        assert h["get_Host"].get_string(u) == "example.com"
        assert h["get_Port"].get_i32(u) == 8080
        assert h["get_Path"].get_string(u) == "/path/to/resource"
        assert h["get_Query"].get_string(u) == "?key=value&a=b"
        assert h["get_Fragment"].get_string(u) == "#section"
        assert h["get_UserName"].get_string(u) == "user"
        assert h["get_Password"].get_string(u) == "pass"
        assert h["get_Domain"].get_string(u) == "example.com"

    def test_simple_https(self):
        """Default HTTPS port should be 443."""
        uri_iid, h, uri, *_ = self._make_uri("https://microsoft.com")
        u = uri.cast(uri_iid)
        assert h["get_Port"].get_i32(u) == 443
        assert h["get_Suspicious"].get_bool(u) is False
        assert h["get_SchemeName"].get_string(u) == "https"

    def test_http_vs_https_port(self):
        """HTTP default port is 80, HTTPS is 443."""
        _, h1, uri1, *_ = self._make_uri("http://example.com")
        _, h2, uri2, *_ = self._make_uri("https://example.com")
        # Both use the same IID so cast works
        iid = WinGUID.parse(self.URI_IID)
        assert h1["get_Port"].get_i32(uri1.cast(iid)) == 80
        assert h2["get_Port"].get_i32(uri2.cast(iid)) == 443

    def test_absolute_uri_roundtrip(self):
        """AbsoluteUri should normalize the URL."""
        uri_iid, h, uri, *_ = self._make_uri("HTTPS://Example.COM/Path")
        u = uri.cast(uri_iid)
        abs_uri = h["get_AbsoluteUri"].get_string(u)
        assert "example.com" in abs_uri.lower()
        assert "/Path" in abs_uri  # path is case-sensitive

    def test_uri_equals(self):
        """Two URIs with same URL should be equal."""
        uri_iid, h, uri1, factory_iid, factory_h, factory = self._make_uri("https://example.com/path")
        uri2 = factory_h["CreateUri"].invoke(factory.cast(factory_iid), [DynWinRTValue.from_hstring("https://example.com/path")])
        
        u1 = uri1.cast(uri_iid)
        equals = h["Equals"].invoke(u1, [uri2])
        assert equals.to_bool() is True

    def test_uri_not_equals(self):
        """Two different URIs should not be equal."""
        uri_iid, h, uri1, factory_iid, factory_h, factory = self._make_uri("https://a.com")
        uri2 = factory_h["CreateUri"].invoke(factory.cast(factory_iid), [DynWinRTValue.from_hstring("https://b.com")])
        
        u1 = uri1.cast(uri_iid)
        equals = h["Equals"].invoke(u1, [uri2])
        assert equals.to_bool() is False
