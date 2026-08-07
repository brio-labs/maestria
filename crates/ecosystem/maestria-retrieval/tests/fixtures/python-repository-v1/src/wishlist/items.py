"""Item model."""


class Item:
    def __init__(self, name, price):
        self.name = name
        self.price = price

    def total(self, quantity):
        return self.price * quantity


def make_item(name, price=1):
    return Item(name, price)
