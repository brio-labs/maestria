import wishlist.items as items
from wishlist.pricing import compute_discount


def create_order(item, quantity):
    total = item.total(quantity)
    discount = compute_discount(total)
    return total - discount
