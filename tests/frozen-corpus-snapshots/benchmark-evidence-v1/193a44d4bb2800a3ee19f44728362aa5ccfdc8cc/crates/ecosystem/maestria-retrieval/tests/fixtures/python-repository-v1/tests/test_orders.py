from wishlist.orders import create_order
from wishlist.items import Item


def test_create_order():
    item = Item("widget", 3)
    total = create_order(item, 2)
    assert total == 5
