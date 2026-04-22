from src.readers.signature_text_parser import parse_signature_text


def test_parse_signature_text_extracts_signature_phone_and_date():
    parsed = parse_signature_text("签字/盖章：黄汉民 电话：14714691425 2026年4月10日")

    assert parsed.signer_name == "黄汉民"
    assert parsed.signer_phone == "14714691425"
    assert parsed.sign_date.isoformat() == "2026-04-10"


def test_parse_signature_text_stops_at_second_signature_label():
    parsed = parse_signature_text("签字/盖章：黄汉民 签字/盖章：（项目经理签字） 电话：14714691425 2026年4月10日")

    assert parsed.signer_name == "黄汉民"
